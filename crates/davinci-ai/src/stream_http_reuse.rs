//! Experiment, opt-in: reuse one HTTPS connection per conversation (`transport: "http-reuse"`).
//!
//! The default path opens a new TCP and TLS connection for every provider request. The 2026-10
//! WebSocket A/B showed later-request cache reuse near 0.6 over that path and near 0.9 on one
//! persistent socket, so this tests whether the connection alone explains the gap. Request
//! bodies, headers and retry rules are unchanged; only the connection is kept between requests.
//!
//! Safety properties that the one-shot path gets for free, kept here:
//! - a cancellation belongs to one request: every request installs its own cancellation in the
//!   session, and a transport that ever saw a failed check is poisoned for good, so a cancelled
//!   or aborted response can never hand its socket to the next request;
//! - one request at a time per conversation: a second concurrent request gets its own one-shot
//!   connection and never shares or evicts the session's connection;
//! - a connection the server closed while idle is probed away by the pool before it is used.
//!
//! The label on the provider attempt says what the request expected: `http-reuse` when the
//! previous response on this conversation ended cleanly less than `MAX_IDLE_AGE` ago (a pooled
//! connection should carry it), `http-reuse-open` when a new connection is expected (first
//! request, after an error or cancel, after an idle gap, or a concurrent request).
use crate::stream_http::Cancellation;
use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ureq_stream::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, NextTimeout, RustlsConnector, TcpConnector, Transport,
};

pub(crate) const TRANSPORT: &str = "http-reuse";
pub(crate) const LABEL_REUSE: &str = "http-reuse";
pub(crate) const LABEL_OPEN: &str = "http-reuse-open";
/// Shorter than the typical server idle timeout; the pool also probes before every reuse.
pub(crate) const MAX_IDLE_AGE: Duration = Duration::from_secs(55);
const MAX_SESSIONS: usize = 64;
const CANCEL_POLL: Duration = Duration::from_millis(100);
/// Longest wait, and most bytes, spent reading the tail of a finished response.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);
const DRAIN_MAX_BYTES: usize = 64 * 1024;
/// How long the next request waits for the previous response's tail to be drained.
const SETTLE_WAIT: Duration = Duration::from_millis(500);

/// True only for the documented public plan route and the exact experimental value.
pub(crate) fn selected(model: &crate::Model, transport: Option<&str>) -> bool {
    crate::openai_siwc::is_public_plan_model(model) && transport == Some(TRANSPORT)
}

struct Current {
    cancellation: Cancellation,
    idle: Duration,
}

struct Shared {
    current: Mutex<Current>,
    connects: AtomicU64,
}

impl Shared {
    fn check(&self) -> Result<(), ureq_stream::Error> {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cancellation
            .check_unless_finished()
    }
    /// The in-flight (or last) request reached its own terminal event; only its tail is left.
    fn finished(&self) -> bool {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cancellation
            .is_finished()
    }
    /// Idle limit for the next read. After the stream's own terminal event only the end of the
    /// body is left, so a server that does not finish it must not hold the connection long.
    fn idle(&self) -> Duration {
        let current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current.cancellation.is_finished() {
            current.idle.min(DRAIN_LIMIT)
        } else {
            current.idle
        }
    }
}

pub(crate) struct Session {
    agent: ureq_stream::Agent,
    shared: Arc<Shared>,
    busy: AtomicBool,
    last_clean: Mutex<Option<Instant>>,
}

impl Session {
    fn new(budgeted: bool) -> Self {
        let shared = Arc::new(Shared {
            current: Mutex::new(Current {
                cancellation: Cancellation::default(),
                idle: crate::http::PROVIDER_IDLE_TIMEOUT,
            }),
            connects: AtomicU64::new(0),
        });
        let config = ureq_stream::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .max_redirects(if budgeted { 0 } else { 5 })
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_resolve(Some(Duration::from_secs(30)))
            .timeout_send_request(Some(Duration::from_secs(60)))
            .timeout_send_body(Some(Duration::from_secs(60)))
            .max_idle_connections(1)
            .max_idle_connections_per_host(1)
            .max_idle_age(MAX_IDLE_AGE)
            .build();
        let connector = TcpConnector::default()
            .chain(ReuseConnector {
                shared: shared.clone(),
            })
            .chain(RustlsConnector::default());
        let agent = ureq_stream::Agent::with_parts(
            config,
            connector,
            ureq_stream::unversioned::resolver::DefaultResolver::default(),
        );
        Self {
            agent,
            shared,
            busy: AtomicBool::new(false),
            last_clean: Mutex::new(None),
        }
    }

    /// Connections this session has opened since it was created.
    #[cfg(test)]
    pub(crate) fn connects(&self) -> u64 {
        self.shared.connects.load(Ordering::Relaxed)
    }
}

/// Held for one request; releases the session when the response reader is dropped.
pub(crate) struct Lease {
    session: Arc<Session>,
    clean: Arc<AtomicBool>,
}

impl Lease {
    pub(crate) fn agent(&self) -> ureq_stream::Agent {
        self.session.agent.clone()
    }
    pub(crate) fn clean_flag(&self) -> Arc<AtomicBool> {
        self.clean.clone()
    }
    /// Installs this request's cancellation; must run before the request is sent.
    pub(crate) fn install(&self, cancellation: Cancellation, idle: Duration) {
        *self
            .session
            .shared
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Current { cancellation, idle };
    }
    #[cfg(test)]
    pub(crate) fn session(&self) -> Arc<Session> {
        self.session.clone()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let clean = self.clean.load(Ordering::Relaxed);
        *self
            .session
            .last_clean
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = clean.then(Instant::now);
        self.session.busy.store(false, Ordering::Release);
    }
}

pub(crate) struct Plan {
    pub(crate) label: &'static str,
    pub(crate) lease: Option<Lease>,
}

impl Plan {
    /// The ordinary connection-per-request path under its own label.
    pub(crate) fn one_shot(label: &'static str) -> Self {
        Self { label, lease: None }
    }
}

type SessionMap = HashMap<(String, bool), Arc<Session>>;

fn sessions() -> &'static Mutex<SessionMap> {
    static SESSIONS: OnceLock<Mutex<SessionMap>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Chooses the connection for the next request of one conversation. Without a conversation id,
/// or while another request of the conversation is in flight, the request gets the ordinary
/// one-shot connection and the `http-reuse-open` label.
pub(crate) fn plan(conversation: Option<&str>) -> Plan {
    let open = Plan {
        label: LABEL_OPEN,
        lease: None,
    };
    let Some(conversation) = conversation.filter(|id| !id.is_empty()) else {
        return open;
    };
    let budgeted = crate::provider_observation::active_budget().is_some();
    let session = {
        let mut map = sessions().lock().unwrap_or_else(|error| error.into_inner());
        if map.len() >= MAX_SESSIONS && !map.contains_key(&(conversation.to_string(), budgeted)) {
            map.retain(|_, session| session.busy.load(Ordering::Acquire));
        }
        map.entry((conversation.to_string(), budgeted))
            .or_insert_with(|| Arc::new(Session::new(budgeted)))
            .clone()
    };
    let acquire = || {
        session
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    };
    // A response that just finished may still be handing its connection back; a request that is
    // really in flight has not reached its terminal event and is never waited for.
    let waited = Instant::now();
    while !acquire() {
        if !session.shared.finished() || waited.elapsed() >= SETTLE_WAIT {
            return open;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let expect_reuse = session
        .last_clean
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some_and(|at| at.elapsed() < MAX_IDLE_AGE);
    Plan {
        label: if expect_reuse {
            LABEL_REUSE
        } else {
            LABEL_OPEN
        },
        lease: Some(Lease {
            session,
            clean: Arc::new(AtomicBool::new(false)),
        }),
    }
}

/// Wraps a response body so the lease is released, and marked clean only after a full read.
pub(crate) struct LeasedReader<R: Read> {
    inner: R,
    lease: Option<(Lease, Arc<AtomicBool>)>,
    cancellation: Cancellation,
}

impl<R: Read> LeasedReader<R> {
    pub(crate) fn new(inner: R, lease: Option<Lease>, cancellation: Cancellation) -> Self {
        Self {
            inner,
            lease: lease.map(|lease| {
                let flag = lease.clean_flag();
                (lease, flag)
            }),
            cancellation,
        }
    }
}

impl<R: Read> Read for LeasedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read == 0 && !buf.is_empty() {
            if let Some((_, flag)) = &self.lease {
                flag.store(true, Ordering::Relaxed);
            }
        }
        Ok(read)
    }
}

impl<R: Read> Drop for LeasedReader<R> {
    /// The consumer stops at the stream's terminal event, before the chunk terminator arrives.
    /// A reader that finished normally reads that tail, bounded in bytes and time, so the
    /// connection can return to the pool; any other drop (cancel, abort, error) leaves the body
    /// unread and the connection is discarded.
    fn drop(&mut self) {
        let Some((_, flag)) = &self.lease else {
            return;
        };
        if flag.load(Ordering::Relaxed) || !self.cancellation.is_finished() {
            return;
        }
        let mut buffer = [0u8; 4096];
        let mut drained = 0;
        while drained < DRAIN_MAX_BYTES {
            match self.inner.read(&mut buffer) {
                Ok(0) => {
                    flag.store(true, Ordering::Relaxed);
                    return;
                }
                Ok(count) => drained += count,
                Err(_) => return,
            }
        }
    }
}

#[derive(Debug)]
struct ReuseConnector {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared")
    }
}

impl<T: Transport> Connector<T> for ReuseConnector {
    type Out = ReuseTransport<T>;
    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<T>,
    ) -> Result<Option<Self::Out>, ureq_stream::Error> {
        self.shared.check()?;
        Ok(chained.map(|inner| {
            self.shared.connects.fetch_add(1, Ordering::Relaxed);
            ReuseTransport {
                inner,
                shared: self.shared.clone(),
                poisoned: false,
            }
        }))
    }
}

#[derive(Debug)]
struct ReuseTransport<T> {
    inner: T,
    shared: Arc<Shared>,
    poisoned: bool,
}

impl<T> ReuseTransport<T> {
    fn guard(&mut self) -> Result<(), ureq_stream::Error> {
        if self.poisoned {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Request aborted").into());
        }
        self.shared.check().inspect_err(|_| self.poisoned = true)
    }
}

impl<T: Transport> Transport for ReuseTransport<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: NextTimeout,
    ) -> Result<(), ureq_stream::Error> {
        self.guard()?;
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq_stream::Error> {
        let started = Instant::now();
        let limit = self.shared.idle().min(*timeout.after);
        loop {
            self.guard()?;
            let remaining = limit.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(ureq_stream::Error::Timeout(timeout.reason));
            }
            let slice = NextTimeout {
                after: remaining.min(CANCEL_POLL).into(),
                reason: timeout.reason,
            };
            match self.inner.await_input(slice) {
                Err(ureq_stream::Error::Timeout(_)) => continue,
                result => return result,
            }
        }
    }
    fn is_open(&mut self) -> bool {
        self.guard().is_ok() && self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[cfg(test)]
#[path = "stream_http_reuse_tests.rs"]
mod tests;

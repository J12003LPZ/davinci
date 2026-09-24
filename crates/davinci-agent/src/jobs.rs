//! Background shell jobs: a command that keeps running while the turn goes
//! on, its output kept for `job_output`, its end announced to the model on
//! its next turn and to the user at once. No TypeScript counterpart; phase 3
//! spec, "Background jobs".
//!
//! The book is shared (`Arc<Mutex<JobBook>>` on the agent): the tool thread
//! writes it, the davinci loop reads it every tick, and the agent loop
//! drains the notices. Dropping the book kills what is still running, so no
//! job outlives the session.

use std::io::Read;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::runtime::operations::AgentOperationHandle;

pub mod managed;
pub mod supervisor;

/// Bytes of output kept per job; above it the head is dropped.
const OUTPUT_CAP: usize = 4 * 1024 * 1024;
/// How much is kept once the cap is passed.
const OUTPUT_KEEP: usize = 3 * 1024 * 1024;
/// Full output retained for the newest finished and announced jobs.
const KEEP_FINISHED_OUTPUT: usize = 16;
/// Lines of output a finished-job notice carries.
pub const NOTICE_LINES: usize = 20;
/// The longest `job_output` will wait for an exit.
const MAX_WAIT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Exited(i32),
    Killed,
}

impl JobStatus {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// `running`, `exit 0`, `killed` — the word on a row.
    pub fn describe(self) -> String {
        match self {
            Self::Running => "running".into(),
            Self::Exited(code) => format!("exit {code}"),
            Self::Killed => "killed".into(),
        }
    }

    pub fn succeeded(self) -> bool {
        self == Self::Exited(0)
    }
}

#[derive(Default)]
struct OutputBuffer {
    bytes: Vec<u8>,
    dropped: bool,
    total_bytes: u64,
    released: bool,
}

impl OutputBuffer {
    fn append(&mut self, chunk: &[u8]) {
        if self.released {
            return;
        }
        self.total_bytes = self.total_bytes.saturating_add(chunk.len() as u64);
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > OUTPUT_CAP {
            let cut = self.bytes.len() - OUTPUT_KEEP;
            // Cut at a line so the kept text starts whole.
            let at = self.bytes[cut..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| cut + offset + 1)
                .unwrap_or(cut);
            self.bytes.drain(..at);
            self.dropped = true;
        }
    }

    fn text(&self) -> String {
        let text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.dropped {
            format!("[earlier output dropped]\n{text}")
        } else {
            text
        }
    }

    fn release(&mut self) {
        self.bytes.clear();
        self.bytes.shrink_to_fit();
        self.released = true;
    }
}

struct Shared {
    status: Mutex<JobStatus>,
    output: Mutex<OutputBuffer>,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    finished_at: Mutex<Option<Instant>>,
    supports_stdin: bool,
    supervisor: Mutex<Option<Arc<supervisor::Supervisor>>>,
}

pub struct Job {
    pub id: u32,
    pub command: String,
    pub pid: u32,
    pub started: Instant,
    pub task_id: Option<crate::runtime::ids::TaskId>,
    pub agent_id: Option<crate::runtime::ids::AgentId>,
    pub generation: Option<u64>,
    /// Durable child launch whose result is completed by the terminal job
    /// notice. Keeping the handle with the job prevents a notice from being
    /// lost between process exit and the next model turn.
    operation: Option<AgentOperationHandle>,
    shared: Arc<Shared>,
    managed: Option<Arc<managed::Record>>,
    /// The model has been told this job finished.
    announced: bool,
    /// The user has seen this job finish.
    seen: bool,
}

impl Job {
    pub fn status(&self) -> JobStatus {
        *self
            .shared
            .status
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// Seconds the job ran, or has run so far.
    pub fn elapsed(&self) -> Duration {
        let finished = *self
            .shared
            .finished_at
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        finished
            .map(|at| at.saturating_duration_since(self.started))
            .unwrap_or_else(|| self.started.elapsed())
    }

    /// Everything the job has printed so far, or its last `tail` lines.
    pub fn output(&self, tail: Option<usize>) -> String {
        let output = self
            .shared
            .output
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let text = if output.released {
            released_output_message(self.id)
        } else {
            output.text()
        };
        match tail {
            Some(n) => {
                let lines: Vec<&str> = text.lines().collect();
                let start = lines.len().saturating_sub(n);
                lines[start..].join("\n")
            }
            None => text,
        }
    }

    fn output_released(&self) -> bool {
        self.shared
            .output
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .released
    }

    fn release_output(&self) {
        self.shared
            .output
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .release();
    }

    pub fn write_stdin(&self, text: &str) -> Result<usize, String> {
        if !self.status().is_running() {
            return Err(format!("Job {} has already exited.", self.id));
        }
        let supervised = self
            .shared
            .supervisor
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(supervised) = supervised {
            return supervised.write(text.as_bytes());
        }
        if !self.shared.supports_stdin {
            return Err(format!(
                "Job {} does not support stdin: stdin is null or closed.",
                self.id
            ));
        }
        let mut guard = self
            .shared
            .stdin
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let stdin = guard
            .as_mut()
            .ok_or_else(|| format!("Job {} stdin is null or closed.", self.id))?;
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
            .map_err(|err| format!("Failed to write to stdin for job {}: {err}", self.id))?;
        stdin
            .flush()
            .map_err(|err| format!("Failed to flush stdin for job {}: {err}", self.id))?;
        Ok(text.len())
    }

    fn kill(&self) -> bool {
        if let Some(record) = &self.managed {
            record.stop();
        }
        if let Some(supervised) = self
            .shared
            .supervisor
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            supervised.stop();
            return true;
        }
        let mut child = self
            .shared
            .child
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(child) = child.as_mut() else {
            return false;
        };
        kill_tree(self.pid);
        let _ = child.kill();
        let mut stdin = self
            .shared
            .stdin
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        drop(stdin.take());
        let mut status = self
            .shared
            .status
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if status.is_running() {
            *status = JobStatus::Killed;
            *self
                .shared
                .finished_at
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = Some(Instant::now());
        }
        true
    }
}

/// Canonical stop lifecycle progression: requested -> stopping -> stopped | failed_to_stop.
pub fn stop_status(requested: bool, exited: bool, failed: bool) -> &'static str {
    if exited {
        "stopped"
    } else if failed {
        "failed_to_stop"
    } else if requested {
        "stopping"
    } else {
        "running"
    }
}

/// `taskkill /T` on Windows, the process group elsewhere; `Child::kill`
/// alone would leave a shell's children running.
pub fn kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let _ = Command::new("kill")
            // A negative PID is a process group, not another signal option.
            .args(["-TERM", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// A job that finished and has not been announced yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobNotice {
    pub id: u32,
    pub command: String,
    pub status: JobStatus,
    pub elapsed: Duration,
    /// The last `NOTICE_LINES` lines.
    pub tail: Vec<String>,
}

impl JobNotice {
    /// `[background job 1 finished · exit 0 · 31.2s] cargo build` and the
    /// tail, as the model reads it.
    pub fn message_text(&self) -> String {
        let mut out = format!(
            "[background job {} finished · {} · {}] {}",
            self.id,
            self.status.describe(),
            format_elapsed(self.elapsed),
            self.command
        );
        if self.tail.is_empty() {
            out.push_str("\n(no output)");
        } else {
            for line in &self.tail {
                out.push('\n');
                out.push_str("    ");
                out.push_str(line);
            }
        }
        out
    }

    /// Convert a finished background job notice into an immutable execution receipt.
    pub fn to_execution_receipt(
        &self,
        task_id: Option<crate::runtime::ids::TaskId>,
    ) -> crate::runtime::evidence_store::ExecutionReceipt {
        let (exit_code, killed) = match self.status {
            JobStatus::Exited(code) => (Some(code), false),
            JobStatus::Killed => (None, true),
            JobStatus::Running => (None, false),
        };
        crate::runtime::evidence_store::ExecutionReceipt {
            receipt_id: crate::runtime::ids::EvidenceId::new(),
            operation_id: format!("job_{}", self.id),
            task_id,
            tool_name: "background_job".into(),
            argv: vec![self.command.clone()],
            cwd: ".".into(),
            started: true,
            exit_code,
            timed_out: false,
            cancelled: killed,
            killed,
            permission_denied: false,
            simulated: false,
            hook_vetoed: false,
            stdout_hash: None,
            stderr_hash: None,
            stdout_artifact: None,
            stderr_artifact: None,
            assertion_counts: None,
            runtime_versions: std::collections::HashMap::new(),
            started_at_ms: 0,
            finished_at_ms: self.elapsed.as_millis() as i64,
            ..Default::default()
        }
    }
}

/// One row of `/jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSummary {
    pub id: u32,
    pub command: String,
    pub status: JobStatus,
    pub elapsed: Duration,
}

/// Every job any book in the process has started, weakly: a signal exit
/// goes through `process::exit`, which runs no `Drop`, so the reaper walks
/// this list instead of the books.
static LIVE_JOBS: Mutex<Vec<(u32, std::sync::Weak<Shared>)>> = Mutex::new(Vec::new());

/// Kill every job still running in this process. For exits that skip
/// destructors (a signal, a hung turn); a normal return drops the books.
pub fn kill_every_job() {
    let live = std::mem::take(&mut *LIVE_JOBS.lock().unwrap_or_else(|err| err.into_inner()));
    for (pid, weak) in live {
        let Some(shared) = weak.upgrade() else {
            continue;
        };
        if let Some(supervised) = shared
            .supervisor
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            supervised.stop();
            continue;
        }
        if !shared
            .status
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_running()
        {
            continue;
        }
        kill_tree(pid);
        let mut guard = shared.child.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
        }
    }
}

#[derive(Default)]
pub struct JobBook {
    jobs: Vec<Job>,
    next_id: u32,
    managed_starts: std::collections::BTreeMap<String, Arc<managed::Flight>>,
    /// The host's active runtime; updated and checked under the book mutex.
    pub(crate) cancellation_owner: Option<std::sync::Weak<std::sync::atomic::AtomicBool>>,
}

impl JobBook {
    /// Register a spawned child. Two threads drain its pipes into the
    /// buffer, a third notices when it exits.
    pub fn register(&mut self, command: &str, child: Child) -> u32 {
        self.register_with_provenance(command, child, None, None, None)
    }

    /// Register a spawned child with task, agent, and generation provenance.
    pub fn register_with_provenance(
        &mut self,
        command: &str,
        child: Child,
        task_id: Option<crate::runtime::ids::TaskId>,
        agent_id: Option<crate::runtime::ids::AgentId>,
        generation: Option<u64>,
    ) -> u32 {
        self.register_with_provenance_and_operation(
            command, child, task_id, agent_id, generation, None,
        )
    }

    /// Register a spawned child and retain its durable launch operation until
    /// the process reaches a terminal state.
    pub fn register_with_provenance_and_operation(
        &mut self,
        command: &str,
        mut child: Child,
        task_id: Option<crate::runtime::ids::TaskId>,
        agent_id: Option<crate::runtime::ids::AgentId>,
        generation: Option<u64>,
        operation: Option<AgentOperationHandle>,
    ) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        let pid = child.id();
        let stdin = child.stdin.take();
        let supports_stdin = stdin.is_some();
        let shared = Arc::new(Shared {
            status: Mutex::new(JobStatus::Running),
            output: Mutex::new(OutputBuffer::default()),
            child: Mutex::new(None),
            stdin: Mutex::new(stdin),
            finished_at: Mutex::new(None),
            supports_stdin,
            supervisor: Mutex::new(None),
        });
        {
            let mut live = LIVE_JOBS.lock().unwrap_or_else(|err| err.into_inner());
            live.retain(|(_, weak)| weak.strong_count() > 0);
            live.push((pid, Arc::downgrade(&shared)));
        }
        for pipe in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut pipe = pipe;
                let mut buf = [0u8; 8192];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => shared
                            .output
                            .lock()
                            .unwrap_or_else(|err| err.into_inner())
                            .append(&buf[..n]),
                    }
                }
            });
        }
        *shared.child.lock().unwrap_or_else(|err| err.into_inner()) = Some(child);
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || loop {
                let exited = {
                    let mut child = shared.child.lock().unwrap_or_else(|err| err.into_inner());
                    match child.as_mut().map(|child| child.try_wait()) {
                        Some(Ok(Some(status))) => Some(status.code().unwrap_or(-1)),
                        Some(Ok(None)) => None,
                        Some(Err(_)) | None => Some(-1),
                    }
                };
                if let Some(code) = exited {
                    let mut status = shared.status.lock().unwrap_or_else(|err| err.into_inner());
                    if status.is_running() {
                        *status = JobStatus::Exited(code);
                        *shared
                            .finished_at
                            .lock()
                            .unwrap_or_else(|err| err.into_inner()) = Some(Instant::now());
                    }
                    drop(
                        shared
                            .stdin
                            .lock()
                            .unwrap_or_else(|err| err.into_inner())
                            .take(),
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            });
        }
        if davinci_ai::trace::enabled() {
            davinci_ai::trace::log(&format!("job {id} start pid={pid} {command}"));
        }
        self.jobs.push(Job {
            id,
            command: command.to_string(),
            pid,
            started: Instant::now(),
            task_id,
            agent_id,
            generation,
            operation,
            shared,
            managed: None,
            announced: false,
            seen: false,
        });
        // Cancellation may have drained the book between spawn and registration.
        // Registration and the cancellation callback share the caller's book lock.
        if self
            .cancellation_owner
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|owner| owner.load(std::sync::atomic::Ordering::SeqCst))
        {
            self.kill(id);
        }
        id
    }

    pub fn kill_jobs_for_agent(&mut self, agent_id: &crate::runtime::ids::AgentId) -> Vec<u32> {
        let matching: Vec<u32> = self
            .jobs
            .iter()
            .filter(|j| j.agent_id.as_ref() == Some(agent_id) && j.status().is_running())
            .map(|j| j.id)
            .collect();
        for id in &matching {
            self.kill(*id);
        }
        matching
    }

    pub fn kill_jobs_for_task(&mut self, task_id: &crate::runtime::ids::TaskId) -> Vec<u32> {
        let matching: Vec<u32> = self
            .jobs
            .iter()
            .filter(|j| j.task_id.as_ref() == Some(task_id) && j.status().is_running())
            .map(|j| j.id)
            .collect();
        for id in &matching {
            self.kill(*id);
        }
        matching
    }

    pub fn get_process_lease(
        &self,
        agent_id: &crate::runtime::ids::AgentId,
    ) -> Option<crate::runtime::control::ProcessLease> {
        let job = self
            .jobs
            .iter()
            .find(|j| j.agent_id.as_ref() == Some(agent_id) && j.status().is_running())?;
        Some(crate::runtime::control::ProcessLease {
            lease_id: uuid::Uuid::new_v4(),
            task_id: job.task_id,
            agent_id: *agent_id,
            generation: job.generation.unwrap_or(1),
            os_handle_identity: job.pid,
            created_at: job.started.elapsed().as_millis() as i64,
            child_tree: vec![job.pid],
        })
    }

    pub fn get(&self, id: u32) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    pub fn ids(&self) -> Vec<u32> {
        self.jobs.iter().map(|job| job.id).collect()
    }

    pub fn running(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.status().is_running())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn kill(&mut self, id: u32) -> Option<JobStatus> {
        let job = self.jobs.iter().find(|job| job.id == id)?;
        job.kill();
        Some(job.status())
    }

    pub fn write_stdin(&self, id: u32, text: &str) -> Result<usize, String> {
        let job = self.get(id).ok_or_else(|| unknown_job(self, id))?;
        job.write_stdin(text)
    }

    pub fn kill_all(&mut self) {
        for job in &self.jobs {
            if job.status().is_running() {
                job.kill();
            }
        }
    }

    pub fn summaries(&self) -> Vec<JobSummary> {
        self.jobs
            .iter()
            .map(|job| JobSummary {
                id: job.id,
                command: job.command.clone(),
                status: job.status(),
                elapsed: job.elapsed(),
            })
            .collect()
    }

    fn notice_of(job: &Job) -> JobNotice {
        JobNotice {
            id: job.id,
            command: job.command.clone(),
            status: job.status(),
            elapsed: job.elapsed(),
            tail: job
                .output(Some(NOTICE_LINES))
                .lines()
                .map(|line| line.trim_end().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        }
    }

    /// Finished jobs the model has not been told about, marked told.
    pub fn take_unannounced(&mut self) -> Vec<JobNotice> {
        let mut out = Vec::new();
        for job in self.jobs.iter_mut() {
            if !job.announced && !job.status().is_running() {
                let notice = Self::notice_of(job);
                if let Some(operation) = job.operation.as_ref() {
                    let result = crate::ToolResult {
                        content: notice.message_text(),
                        is_error: !notice.status.succeeded(),
                        details: Some(json!({
                            "jobId": notice.id,
                            "command": notice.command,
                            "status": notice.status.describe(),
                            "elapsedMs": notice.elapsed.as_millis(),
                            "tail": notice.tail,
                        })),
                    };
                    if let Err(error) = operation.complete(&result) {
                        if davinci_ai::trace::enabled() {
                            davinci_ai::trace::log(&format!(
                                "job {} operation completion deferred: {error}",
                                job.id
                            ));
                        }
                        continue;
                    }
                    job.operation = None;
                }
                job.announced = true;
                if davinci_ai::trace::enabled() {
                    davinci_ai::trace::log(&format!("job {} {}", job.id, job.status().describe()));
                }
                out.push(notice);
            }
        }
        self.release_old_output();
        out
    }

    fn release_old_output(&mut self) {
        let mut finished: Vec<usize> = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.announced && !job.status().is_running())
            .map(|(index, _)| index)
            .collect();
        if finished.len() <= KEEP_FINISHED_OUTPUT {
            return;
        }
        finished.truncate(finished.len() - KEEP_FINISHED_OUTPUT);
        for index in finished {
            self.jobs[index].release_output();
        }
    }

    /// Finished jobs the user has not seen finish, marked seen.
    pub fn take_unseen(&mut self) -> Vec<JobNotice> {
        let mut out = Vec::new();
        for job in self.jobs.iter_mut() {
            if !job.seen && !job.status().is_running() {
                job.seen = true;
                out.push(Self::notice_of(job));
            }
        }
        out
    }
}

impl Drop for JobBook {
    fn drop(&mut self) {
        self.kill_all();
    }
}

impl std::fmt::Debug for JobBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobBook")
            .field("jobs", &self.ids())
            .field("running", &self.running())
            .finish()
    }
}

/// `12.4s`, `2m 05s`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let whole = elapsed.as_secs();
        format!("{}m {:02}s", whole / 60, whole % 60)
    }
}

pub fn job_id(input: &Value) -> Result<u32, String> {
    let raw = input
        .get("jobId")
        .or_else(|| input.get("job_id"))
        .or_else(|| input.get("id"))
        .or_else(|| input.get("job"))
        .ok_or("Missing jobId")?;
    match raw {
        Value::Number(n) => n
            .as_u64()
            .map(|n| n as u32)
            .ok_or_else(|| "jobId must be a positive integer".into()),
        Value::String(s) => s
            .trim()
            .trim_start_matches("job ")
            .parse::<u32>()
            .map_err(|_| format!("jobId must be a positive integer, not `{s}`")),
        _ => Err("jobId must be a positive integer".into()),
    }
}

fn unknown_job(book: &JobBook, id: u32) -> String {
    let ids = book.ids();
    if ids.is_empty() {
        format!("No background job {id}: no jobs have been started in this session.")
    } else {
        format!(
            "No background job {id}. Known jobs: {}.",
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn released_output_message(id: u32) -> String {
    format!("Output of job {id} was released; rerun the command to see it again.")
}

/// The reply to a background `bash`: which job, and how to reach it.
pub fn started_result(id: u32, pid: u32, command: &str) -> crate::ToolResult {
    crate::ToolResult {
        content: format!(
            "Started background job {id}: `{command}`\nRead its output with job_output {{\"jobId\": {id}}}; stop it with job_kill.",
        ),
        is_error: false,
        details: Some(json!({"jobId": id, "pid": pid, "command": command})),
    }
}

/// `job_output { jobId, wait?, tail? }`. `abort` ends a wait early when the
/// turn is interrupted; the job itself keeps running.
pub fn output_tool(
    book: &Arc<Mutex<JobBook>>,
    input: &Value,
    abort: Option<&std::sync::atomic::AtomicBool>,
) -> Result<crate::ToolResult, String> {
    let id = job_id(input)?;
    let wait = input
        .get("wait")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(|seconds| Duration::from_secs_f64(seconds).min(MAX_WAIT));
    let tail = input
        .get("tail")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0);
    let shared = {
        let book = book.lock().unwrap_or_else(|err| err.into_inner());
        let job = book.get(id).ok_or_else(|| unknown_job(&book, id))?;
        if job.managed.is_some() {
            return Err("Use process_output with the managed process owner".into());
        }
        Arc::clone(&job.shared)
    };
    if let Some(limit) = wait {
        let deadline = Instant::now() + limit;
        while shared
            .status
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_running()
            && Instant::now() < deadline
            && !abort.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let book = book.lock().unwrap_or_else(|err| err.into_inner());
    let job = book.get(id).ok_or_else(|| unknown_job(&book, id))?;
    if job.output_released() {
        return Err(released_output_message(id));
    }
    let status = job.status();
    let output = job.output(tail);
    let elapsed = format_elapsed(job.elapsed());
    let body = if output.trim().is_empty() {
        "(no output yet)".to_string()
    } else {
        output
    };
    let lines = body.lines().count();
    Ok(crate::ToolResult {
        content: format!("{body}\n\n[job {id} {} · {elapsed}]", status.describe()),
        is_error: false,
        details: Some(json!({
            "jobId": id,
            "status": status.describe(),
            "running": status.is_running(),
            "exitCode": match status { JobStatus::Exited(code) => Some(code), _ => None },
            "elapsed": elapsed,
            "lines": lines,
        })),
    })
}

/// `job_kill { jobId }`.
pub fn kill_tool(book: &Arc<Mutex<JobBook>>, input: &Value) -> Result<crate::ToolResult, String> {
    let id = job_id(input)?;
    let mut book = book.lock().unwrap_or_else(|err| err.into_inner());
    if book.get(id).is_some_and(|job| job.managed.is_some()) {
        return Err("Use process_stop with the managed process owner".into());
    }
    let before = book
        .get(id)
        .map(|job| job.status())
        .ok_or_else(|| unknown_job(&book, id))?;
    let status = book.kill(id).unwrap_or(before);
    let job = book.get(id).ok_or_else(|| unknown_job(&book, id))?;
    let elapsed = format_elapsed(job.elapsed());
    let content = if before.is_running() {
        format!(
            "Killed background job {id} (`{}`) after {elapsed}.",
            job.command
        )
    } else {
        format!(
            "Background job {id} (`{}`) had already finished: {} after {elapsed}.",
            job.command,
            before.describe()
        )
    };
    Ok(crate::ToolResult {
        content,
        is_error: false,
        details: Some(json!({"jobId": id, "status": status.describe(), "elapsed": elapsed})),
    })
}

pub fn output_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "jobId": {"type": "integer", "description": "The job number bash returned"},
            "wait": {"type": "number", "description": "Seconds to wait for the job to exit before answering (optional, max 600)"},
            "tail": {"type": "integer", "description": "Return only the last N lines (optional)"}
        },
        "required": ["jobId"]
    })
}

pub fn kill_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "jobId": {"type": "integer", "description": "The job number bash returned"}
        },
        "required": ["jobId"]
    })
}

/// `write_stdin { jobId / job_id, input }`.
pub fn stdin_tool(book: &Arc<Mutex<JobBook>>, input: &Value) -> Result<crate::ToolResult, String> {
    let id = job_id(input)?;
    let text = input
        .get("input")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing required parameter: input".to_string())?;
    let book = book.lock().unwrap_or_else(|err| err.into_inner());
    if book.get(id).is_some_and(|job| job.managed.is_some()) {
        return Err("Use process_write with the managed process owner".into());
    }
    let bytes = book.write_stdin(id, text)?;
    Ok(crate::ToolResult {
        content: format!("Sent {bytes} bytes to stdin"),
        is_error: false,
        details: Some(json!({ "jobId": id, "bytes": bytes })),
    })
}

pub fn stdin_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": {"type": "integer", "description": "The job ID returned when the background command started"},
            "input": {"type": "string", "description": "The input text to send to stdin"}
        },
        "required": ["job_id", "input"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f03_replaced_runtime_cannot_kill_current_jobs() {
        let mut agent = crate::Agent::new("job binding fixture");
        let old = crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        );
        agent.set_runtime(old.clone());
        let current = crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        );
        agent.set_runtime(current.clone());
        let mut command = if cfg!(windows) {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::In.ReadLine()",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "read line"]);
            command
        };
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let jobs = agent.tool_context.jobs.clone();
        let id = jobs
            .lock()
            .unwrap()
            .register("fixture waiting for input", child);
        old.cancellation_token.cancel();
        assert_eq!(
            jobs.lock().unwrap().get(id).unwrap().status(),
            JobStatus::Running
        );
        current.cancellation_token.cancel();
        assert_eq!(
            jobs.lock().unwrap().get(id).unwrap().status(),
            JobStatus::Killed
        );
        let late_child = command.spawn().unwrap();
        let late_id = jobs
            .lock()
            .unwrap()
            .register("late cancellation fixture", late_child);
        assert_eq!(
            jobs.lock().unwrap().get(late_id).unwrap().status(),
            JobStatus::Killed
        );
        for job_id in [id, late_id] {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let book = jobs.lock().unwrap();
                let mut child = book.get(job_id).unwrap().shared.child.lock().unwrap();
                if child.as_mut().unwrap().try_wait().unwrap().is_some() {
                    break;
                }
                drop(child);
                drop(book);
                assert!(
                    Instant::now() < deadline,
                    "cancelled fixture child did not exit"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn spawn(script: &str) -> Child {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            c
        };
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn wait_for_exit(book: &Arc<Mutex<JobBook>>, id: u32) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while book.lock().unwrap().get(id).unwrap().status().is_running() {
            assert!(Instant::now() < deadline, "job {id} never exited");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_finished_job_is_announced_once_with_its_tail() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        let id = book.lock().unwrap().register("echo hi", spawn("echo hi"));
        assert_eq!(id, 1);
        wait_for_exit(&book, id);
        let notices = book.lock().unwrap().take_unannounced();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].status, JobStatus::Exited(0));
        assert_eq!(notices[0].tail, vec!["hi"]);
        let text = notices[0].message_text();
        assert!(
            text.starts_with("[background job 1 finished · exit 0 · "),
            "{text}"
        );
        assert!(text.ends_with("] echo hi\n    hi"), "{text}");
        assert!(book.lock().unwrap().take_unannounced().is_empty());
        // The user's view is a separate ledger.
        assert_eq!(book.lock().unwrap().take_unseen().len(), 1);
        assert!(book.lock().unwrap().take_unseen().is_empty());
    }

    #[test]
    fn old_announced_jobs_release_their_output() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        let payload = "x".repeat(1024);
        for i in 0..20 {
            let command = format!("echo {payload}");
            let id = book
                .lock()
                .unwrap()
                .register(&format!("echo {i}"), spawn(&command));
            wait_for_exit(&book, id);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let total_bytes = book
                    .lock()
                    .unwrap()
                    .get(id)
                    .unwrap()
                    .shared
                    .output
                    .lock()
                    .unwrap()
                    .total_bytes;
                if total_bytes >= payload.len() as u64 {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "job {id} output was not captured"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let notices = book.lock().unwrap().take_unannounced();
        assert_eq!(notices.len(), 20);
        let kept = book
            .lock()
            .unwrap()
            .jobs
            .iter()
            .filter(|job| !job.shared.output.lock().unwrap().bytes.is_empty())
            .count();
        assert_eq!(kept, 16);
        assert_eq!(
            output_tool(&book, &json!({"jobId": 1}), None).unwrap_err(),
            "Output of job 1 was released; rerun the command to see it again."
        );
        assert!(output_tool(&book, &json!({"jobId": 20}), None)
            .unwrap()
            .content
            .contains(&payload));
    }

    #[test]
    fn job_output_waits_for_an_exit_and_reports_the_status() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        let id = book.lock().unwrap().register("echo one", spawn("echo one"));
        let result = output_tool(&book, &json!({"jobId": id, "wait": 10}), None).unwrap();
        assert!(result.content.contains("one"), "{}", result.content);
        assert!(
            result.content.contains(&format!("[job {id} exit 0 · ")),
            "{}",
            result.content
        );
        assert_eq!(result.details.as_ref().unwrap()["exitCode"], 0);
        let missing = output_tool(&book, &json!({"jobId": 9}), None).unwrap_err();
        assert_eq!(missing, "No background job 9. Known jobs: 1.");
        let none = output_tool(
            &Arc::new(Mutex::new(JobBook::default())),
            &json!({"jobId": 1}),
            None,
        )
        .unwrap_err();
        assert!(none.contains("no jobs have been started"));
    }

    #[test]
    #[cfg(unix)]
    fn killing_a_job_group_preserves_the_callers_group() {
        use std::os::unix::process::CommandExt;

        const HELPER: &str = "DAVINCI_TEST_KILL_GROUP_HELPER";
        if std::env::var_os(HELPER).is_some() {
            let mut child = Command::new("sleep")
                .arg("2")
                .process_group(0)
                .spawn()
                .unwrap();
            kill_tree(child.id());
            assert!(!child.wait().unwrap().success());
            return;
        }

        // Isolate the probe so an incorrect group signal fails this assertion
        // instead of terminating cargo and the CI runner's shell.
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "jobs::tests::killing_a_job_group_preserves_the_callers_group",
                "--nocapture",
            ])
            .env(HELPER, "1")
            .process_group(0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "group cleanup terminated or failed its caller: {:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn a_sleeping_job_can_be_killed() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1 > nul"
        } else {
            "sleep 30"
        };
        let id = book.lock().unwrap().register("sleep 30", spawn(script));
        let killed = kill_tool(&book, &json!({"jobId": id})).unwrap();
        assert!(
            killed
                .content
                .starts_with("Killed background job 1 (`sleep 30`)"),
            "{}",
            killed.content
        );
        let status = book.lock().unwrap().get(id).unwrap().status();
        assert!(
            matches!(status, JobStatus::Killed | JobStatus::Exited(_)),
            "{status:?}"
        );
        let again = kill_tool(&book, &json!({"jobId": id})).unwrap();
        assert!(
            again.content.contains("had already finished"),
            "{}",
            again.content
        );
    }

    #[test]
    fn the_buffer_drops_its_head_past_the_cap_and_says_so() {
        let mut buffer = OutputBuffer::default();
        let line = "x".repeat(1023) + "\n";
        for _ in 0..(OUTPUT_CAP / 1024 + 10) {
            buffer.append(line.as_bytes());
        }
        assert!(buffer.dropped);
        assert!(
            buffer.bytes.len() <= OUTPUT_KEEP + 11 * 1024,
            "{}",
            buffer.bytes.len()
        );
        assert!(buffer.text().starts_with("[earlier output dropped]\nxxx"));
    }

    #[test]
    fn elapsed_reads_as_seconds_then_minutes() {
        assert_eq!(format_elapsed(Duration::from_millis(12_400)), "12.4s");
        assert_eq!(format_elapsed(Duration::from_secs(125)), "2m 05s");
    }

    #[test]
    fn job_ids_come_as_numbers_or_words() {
        assert_eq!(job_id(&json!({"jobId": 3})).unwrap(), 3);
        assert_eq!(job_id(&json!({"jobId": "job 4"})).unwrap(), 4);
        assert_eq!(job_id(&json!({"job_id": 5})).unwrap(), 5);
        assert!(job_id(&json!({})).unwrap_err().contains("Missing jobId"));
    }

    #[test]
    fn write_stdin_rejects_null_or_closed_stdin() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        // Spawn with Stdio::null()
        let id = book
            .lock()
            .unwrap()
            .register("echo null_stdin", spawn("echo null_stdin"));
        let err = book
            .lock()
            .unwrap()
            .write_stdin(id, "test input\n")
            .unwrap_err();
        assert!(
            err.contains("does not support stdin")
                || err.contains("already exited")
                || err.contains("null or closed"),
            "{err}"
        );
    }

    #[test]
    fn write_stdin_returns_error_on_missing_or_exited_job() {
        let book = Arc::new(Mutex::new(JobBook::default()));
        let err = book.lock().unwrap().write_stdin(999, "test").unwrap_err();
        assert!(err.contains("No background job 999"), "{err}");

        let id = book
            .lock()
            .unwrap()
            .register("echo quick", spawn("echo quick"));
        wait_for_exit(&book, id);
        let err_exited = book.lock().unwrap().write_stdin(id, "test").unwrap_err();
        assert!(err_exited.contains("has already exited"), "{err_exited}");
    }

    #[test]
    fn write_stdin_delivers_bytes_to_interactive_process() {
        fn spawn_interactive(script: &str) -> Child {
            let mut command = if cfg!(windows) {
                let mut c = Command::new("powershell");
                c.args(["-NoProfile", "-NonInteractive", "-Command", script]);
                c
            } else {
                let mut c = Command::new("sh");
                c.args(["-c", script]);
                c
            };
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::piped())
                .spawn()
                .unwrap()
        }

        let script = if cfg!(windows) {
            "$line = [Console]::In.ReadLine(); Write-Output \"observed: $line\""
        } else {
            "read line && echo \"observed: $line\""
        };

        let book = Arc::new(Mutex::new(JobBook::default()));
        let id = book
            .lock()
            .unwrap()
            .register("interactive", spawn_interactive(script));

        // Use stdin_tool to deliver the input
        let res = stdin_tool(
            &book,
            &json!({"job_id": id, "input": "hello_interactive\n"}),
        )
        .unwrap();
        assert!(res.content.contains("Sent"));
        assert!(!res.is_error);

        wait_for_exit(&book, id);
        let out = book.lock().unwrap().get(id).unwrap().output(None);
        assert!(out.contains("observed: hello_interactive"), "{out}");
    }

    #[test]
    fn f07_stop_requires_exit() {
        assert_eq!(stop_status(true, false, false), "stopping");
        assert_eq!(stop_status(true, true, false), "stopped");
        assert_eq!(stop_status(true, false, true), "failed_to_stop");
    }

    #[test]
    fn test_process_tree_kill_provenance() {
        let mut book = JobBook::default();
        let agent_id = crate::runtime::ids::AgentId::new();
        let task_id = crate::runtime::ids::TaskId::new();
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1 > nul"
        } else {
            "sleep 30"
        };

        let child = spawn(script);
        let pid = child.id();
        let job_id = book.register_with_provenance(
            "sleep 30",
            child,
            Some(task_id),
            Some(agent_id),
            Some(1),
        );

        let lease = book.get_process_lease(&agent_id);
        assert!(lease.is_some());
        let lease = lease.unwrap();
        assert_eq!(lease.agent_id, agent_id);
        assert_eq!(lease.task_id, Some(task_id));
        assert_eq!(lease.os_handle_identity, pid);

        let killed = book.kill_jobs_for_agent(&agent_id);
        assert_eq!(killed, vec![job_id]);
    }

    #[test]
    fn test_stop_status_reducer_cases() {
        // already exited
        assert_eq!(stop_status(true, true, false), "stopped");
        // requested and stopping
        assert_eq!(stop_status(true, false, false), "stopping");
        // access denied / failed
        assert_eq!(stop_status(true, false, true), "failed_to_stop");
        // not requested, still running
        assert_eq!(stop_status(false, false, false), "running");
    }

    #[test]
    fn test_unrelated_process_unaffected() {
        let mut book = JobBook::default();
        let agent1 = crate::runtime::ids::AgentId::new();
        let agent2 = crate::runtime::ids::AgentId::new();
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1 > nul"
        } else {
            "sleep 30"
        };

        let child1 = spawn(script);
        let child2 = spawn(script);

        let id1 = book.register_with_provenance("worker1", child1, None, Some(agent1), Some(1));
        let id2 = book.register_with_provenance("worker2", child2, None, Some(agent2), Some(1));

        let killed = book.kill_jobs_for_agent(&agent1);
        assert_eq!(killed, vec![id1]);

        // id2 is not killed by agent1 kill call
        assert_eq!(book.get(id2).unwrap().agent_id, Some(agent2));
        assert!(book.get(id2).unwrap().status().is_running());
    }
}

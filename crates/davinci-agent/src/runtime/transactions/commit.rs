//! Read-only Git object observation. This module never creates a commit.
use super::{model::*, transition, Authority, TransactionCoordinator};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

impl TransactionCoordinator {
    /// Caller must check current Git metadata authority before using this helper.
    pub(super) fn observed_revision(&self) -> Result<String, String> {
        let top = git(&self.root, &["rev-parse", "--show-toplevel"], 8192)?;
        let top = std::str::from_utf8(&top)
            .map_err(|_| "Git root is not UTF-8")?
            .trim();
        if super::files::root(Path::new(top))? != self.root {
            return Err("Git root does not match transaction workspace".into());
        }
        let revision = git(&self.root, &["rev-parse", "--verify", "HEAD^{commit}"], 128)?;
        let revision = std::str::from_utf8(&revision)
            .map_err(|_| "invalid Git revision")?
            .trim();
        if !valid_revision(revision) {
            return Err("invalid Git revision".into());
        }
        Ok(revision.into())
    }
    /// Trusted host API requiring authority for read-only Git inspection and each
    /// affected source. The commit ID records an immutable observed object, not
    /// a promise that HEAD cannot move after this method returns.
    pub fn observe_commit(
        &self,
        id: &str,
        authority: Authority<'_>,
    ) -> Result<TransactionSummary, String> {
        let record = self.locked(|store| {
            let record = store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if !matches!(
                record.summary.state,
                TransactionState::Applied | TransactionState::Verified
            ) {
                return Err("transaction state does not allow commit observation".into());
            }
            self.check_verification_images(&record, authority)?;
            Ok(record)
        })?;
        // No transaction lock spans Git execution.
        let revision = self.observed_revision()?;
        for change in &record.changes {
            authority(&self.root.join(&change.path))?;
            let literal = format!(":(literal){}", change.path);
            let entry = git(
                &self.root,
                &["ls-tree", "-z", &revision, "--", &literal],
                16 * 1024,
            )?;
            let Some(expected) = change.proposed_bytes.as_deref() else {
                if !entry.is_empty() {
                    return Err("deleted transaction path is still committed".into());
                }
                continue;
            };
            let entry = std::str::from_utf8(&entry).map_err(|_| "invalid Git tree entry")?;
            let (header, path) = entry
                .strip_suffix('\0')
                .and_then(|entry| entry.split_once('\t'))
                .ok_or("transaction path is absent from commit")?;
            let fields: Vec<_> = header.split(' ').collect();
            if path != change.path
                || fields.len() != 3
                || fields[1] != "blob"
                || !valid_revision(fields[2])
            {
                return Err("unexpected Git tree entry".into());
            }
            let mode = if change.proposed.mode.is_some_and(|mode| mode & 0o111 != 0) {
                "100755"
            } else {
                "100644"
            };
            if fields[0] != mode {
                return Err("committed file mode differs from transaction".into());
            }
            let bytes = git(&self.root, &["cat-file", "blob", fields[2]], MAX_FILE_BYTES)?;
            if bytes != expected {
                return Err("committed bytes differ from transaction".into());
            }
        }
        self.locked(|store| {
            let mut current =
                store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if current.summary.sequence != record.summary.sequence {
                return Err("transaction changed during Git observation".into());
            }
            self.check_verification_images(&current, authority)?;
            current.summary.commit_revision = Some(revision);
            transition(&mut current, TransactionState::Committed);
            store.save(&current)?;
            Ok(current.summary)
        })
    }
}

pub(super) fn validate(record: &Record) -> Result<(), String> {
    match (&record.summary.commit_revision, record.summary.state) {
        (Some(revision), TransactionState::Committed) if valid_revision(revision) => Ok(()),
        (None, state) if state != TransactionState::Committed => Ok(()),
        _ => Err("invalid transaction commit evidence".into()),
    }
}

fn valid_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git(root: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command.current_dir(root).env_clear();
    for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        // Missing promisor objects must fail locally, never invoke a remote helper.
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Git observation could not start")?;
    let stdout = child.stdout.take().ok_or("Git stdout unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Git observation failed or timed out");
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "Git output reader failed")?
        .map_err(|_| "Git output read failed")?;
    if !status?.success() || bytes.len() > limit {
        return Err("Git observation failed or exceeded output bound".into());
    }
    Ok(bytes)
}

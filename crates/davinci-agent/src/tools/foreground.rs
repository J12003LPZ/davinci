use super::{ToolContext, ToolError};
use crate::jobs::supervisor::{ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand};
use std::{
    process::Output,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    overflow: bool,
}

impl Capture {
    fn append(&mut self, bytes: &[u8], stderr: bool) {
        let stream = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        let retained = bytes
            .len()
            .min(super::MAX_SHELL_STREAM_BYTES.saturating_sub(stream.len()));
        stream.extend_from_slice(&bytes[..retained]);
        self.overflow |= retained != bytes.len();
    }
}

/// Foreground shell tools have always inherited the host environment. Preserve
/// that contract explicitly across the private helper, without exposing values.
pub(super) fn config(
    cwd: &std::path::Path,
    executable: std::path::PathBuf,
    argv: Vec<String>,
    context: &ToolContext,
) -> Result<ProcessConfig, ToolError> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| ToolError::Failed(format!("command cwd is unavailable: {error}")))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| ToolError::Failed("command executable path is not UTF-8".into()))?;
    let executable = crate::process_manager::resolve_native_executable(executable, &cwd)
        .map_err(ToolError::Failed)?;
    let environment = std::env::vars_os()
        .map(|(key, value)| {
            Ok((
                key.into_string().map_err(|_| {
                    ToolError::Failed("command environment name is not UTF-8".into())
                })?,
                value.into_string().map_err(|_| {
                    ToolError::Failed("command environment value is not UTF-8".into())
                })?,
            ))
        })
        .collect::<Result<_, ToolError>>()?;
    let config = ProcessConfig::new(executable, argv, cwd, environment);
    Ok(context
        .command_receipt
        .as_ref()
        .and_then(|capture| capture.process_operation_binding())
        .map(|binding| config.clone().with_operation_binding(binding))
        .unwrap_or(config))
}

pub(super) fn run(
    host: &SupervisorCommand,
    config: ProcessConfig,
    input: &[u8],
    timeout_ms: Option<u64>,
    timeout_label: Option<&str>,
    context: &ToolContext,
) -> Result<Output, ToolError> {
    let start = Instant::now();
    let started_at_ms = crate::command_receipt::now();
    let interrupted = || {
        if context.is_aborted() {
            Some("Command aborted".to_owned())
        } else if timeout_ms.is_some_and(|ms| start.elapsed() >= Duration::from_millis(ms)) {
            Some(format!(
                "Command timed out after {} seconds",
                timeout_label.unwrap_or("0")
            ))
        } else {
            None
        }
    };
    if let Some(error) = interrupted() {
        return Err(ToolError::Failed(error));
    }
    if let Some(runtime) = &context.runtime {
        let event = crate::runtime::RuntimeEvent::BeforeProcessStart {
            executable: config.executable.to_string_lossy().into_owned(),
            argv: config.argv.clone(),
            cwd: config.cwd.clone(),
        };
        if let Err(reason) = runtime.emit_decision(event) {
            return Err(ToolError::Failed(format!(
                "before-process-start hook blocked: {reason}"
            )));
        }
    }
    let capture = Arc::new(Mutex::new(Capture::default()));
    let stdout = capture.clone();
    let stderr = capture.clone();
    let process = match Supervisor::spawn_with_stderr(
        host,
        config.clone(),
        Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                stdout
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .append(&bytes, false);
            }
        }),
        Arc::new(move |bytes| {
            stderr
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .append(&bytes, true);
        }),
    ) {
        Ok(process) => process,
        Err(error) => {
            if let Some(receipt) = &context.command_receipt {
                receipt.process_observed(error.evidence().clone(), started_at_ms, &[], &[]);
            }
            return Err(ToolError::Failed(error.to_string()));
        }
    };
    let stop = |reason: String| {
        process.stop();
        let exit = process.wait(Duration::from_secs(2));
        let capture = capture.lock().unwrap_or_else(|e| e.into_inner());
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&capture.stdout),
            String::from_utf8_lossy(&capture.stderr)
        );
        let cleanup = if exit.is_some() {
            ""
        } else {
            "; process cleanup did not finish within deadline"
        };
        let evidence = config.execution_evidence(
            process.identity().clone(),
            exit.as_ref()
                .map(|exit| exit.launch_state)
                .unwrap_or(crate::jobs::supervisor::ProcessLaunchState::Unknown),
            exit.as_ref().and_then(|exit| exit.code),
            exit.as_ref()
                .map(|exit| exit.output_complete && !capture.overflow),
        );
        if let Some(receipt) = &context.command_receipt {
            receipt.process_observed(evidence, started_at_ms, &capture.stdout, &capture.stderr);
        }
        ToolError::Failed(format!("{output}\n{reason}{cleanup}"))
    };
    for chunk in input.chunks(16 * 1024) {
        if let Some(error) = interrupted() {
            return Err(stop(error));
        }
        process.write(chunk).map_err(stop)?;
    }
    // A short command can exit before its stdin-close acknowledgement arrives.
    // Its complete exit event is still authoritative; never replay the command.
    if let Err(error) = process.close_stdin() {
        if process.wait(Duration::ZERO).is_none() {
            return Err(stop(error));
        }
    }
    let exit = loop {
        if let Some(error) = interrupted() {
            return Err(stop(error));
        }
        if let Some(exit) = process.wait(Duration::from_millis(10)) {
            break exit;
        }
    };
    let mut captured = capture.lock().unwrap_or_else(|e| e.into_inner());
    let output_complete = exit.output_complete && !captured.overflow;
    let evidence = config.execution_evidence(
        exit.identity.clone(),
        exit.launch_state,
        exit.code,
        Some(output_complete),
    );
    if let Some(receipt) = &context.command_receipt {
        receipt.process_observed(evidence, started_at_ms, &captured.stdout, &captured.stderr);
    }
    if let Some(runtime) = &context.runtime {
        runtime.emit_observe(crate::runtime::RuntimeEvent::AfterProcessExit {
            executable: config.executable.to_string_lossy().into_owned(),
            argv: config.argv.clone(),
            exit_code: exit.code,
            is_error: !output_complete
                || exit.error.is_some()
                || exit.stopped
                || exit.code != Some(0),
        });
    }
    if exit.stopped || exit.error.is_some() || !output_complete {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&captured.stdout),
            String::from_utf8_lossy(&captured.stderr)
        );
        let reason = exit
            .error
            .unwrap_or_else(|| "command output capture incomplete or process stopped".into());
        return Err(ToolError::Failed(if output.is_empty() {
            reason
        } else {
            format!("{output}\n{reason}")
        }));
    }
    if captured.overflow {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&captured.stdout),
            String::from_utf8_lossy(&captured.stderr)
        );
        return Err(ToolError::Failed(format!(
            "{output}\ncommand output exceeded stream byte limit"
        )));
    }
    let code = exit
        .code
        .ok_or_else(|| ToolError::Failed("command exited without an exit code".into()))?;
    #[cfg(unix)]
    let status = {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    };
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    };
    Ok(Output {
        status,
        stdout: std::mem::take(&mut captured.stdout),
        stderr: std::mem::take(&mut captured.stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[test]
    fn foreground_supervisor_fixture() {
        if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
            crate::jobs::supervisor::run();
        }
    }

    #[test]
    fn foreground_pipe_holder_fixture() {
        let Ok(ready) = std::env::var("DAVINCI_FOREGROUND_PIPE_FIXTURE") else {
            return;
        };
        let script = format!("const s=require('net').createServer(c=>c.end());s.listen(0,'127.0.0.1',()=>require('fs').writeFileSync({},JSON.stringify(s.address().port)));setTimeout(()=>process.exit(),10000)", serde_json::to_string(&ready).unwrap());
        let mut child = std::process::Command::new("node")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !std::path::Path::new(&ready).exists() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_none());
        std::process::exit(0);
    }

    #[test]
    fn foreground_supervisor_capture_and_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let host = SupervisorCommand {
            executable: std::env::current_exe().unwrap(),
            argv: vec![
                "--exact".into(),
                "tools::foreground::tests::foreground_supervisor_fixture".into(),
                "--nocapture".into(),
            ],
        };
        let config = |script: &str| {
            ProcessConfig::new(
                "node".into(),
                vec!["-e".into(), script.into()],
                root.path().into(),
                ["PATH", "SystemRoot", "TEMP", "TMP"]
                    .into_iter()
                    .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
                    .collect(),
            )
        };
        let output = run(&host, config("let b='';process.stdin.on('data',s=>b+=s);process.stdin.on('end',()=>{process.stdout.write(b);process.stderr.write('err');process.exitCode=7})"), b"literal input", Some(5_000), Some("5"), &ToolContext::default()).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"literal input");
        assert_eq!(output.stderr, b"err");

        let receipt = crate::command_receipt::CommandReceiptCapture::new(
            "supervised-foreground",
            "exec_command",
            None,
        );
        let context = ToolContext {
            foreground_supervisor: Some(host.clone()),
            command_receipt: Some(receipt.clone()),
            ..Default::default()
        };
        let command = if cfg!(windows) {
            "[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 7"
        } else {
            "printf out; printf err >&2; exit 7"
        };
        let result = crate::tools::execute_tool_with(
            root.path(),
            "exec_command",
            &serde_json::json!({"command":command}),
            &context,
        )
        .unwrap();
        assert!(result.is_error);
        assert_eq!(result.details.unwrap()["exitCode"], 7);
        assert_eq!(result.content, "out\nerr");
        let receipt = receipt
            .take()
            .expect("supervised output must produce a receipt");
        assert_eq!(receipt.exit_code, Some(7));
        assert_eq!(
            receipt.stdout_hash,
            Some(crate::runtime::checkpoints::compute_sha256(b"out"))
        );
        assert_eq!(
            receipt.stderr_hash,
            Some(crate::runtime::checkpoints::compute_sha256(b"err"))
        );

        let ready = root.path().join("listener.json");
        let mut held = config("");
        held.executable = std::env::current_exe().unwrap();
        held.argv = vec![
            "--exact".into(),
            "tools::foreground::tests::foreground_pipe_holder_fixture".into(),
            "--nocapture".into(),
        ];
        held.environment.insert(
            "DAVINCI_FOREGROUND_PIPE_FIXTURE".into(),
            ready.to_string_lossy().into_owned(),
        );
        let incomplete_receipt = crate::command_receipt::CommandReceiptCapture::new(
            "incomplete-output",
            "exec_command",
            None,
        );
        let incomplete_context = ToolContext {
            command_receipt: Some(incomplete_receipt.clone()),
            ..Default::default()
        };
        let start = std::time::Instant::now();
        let error = run(
            &host,
            held,
            b"",
            Some(5_000),
            Some("5"),
            &incomplete_context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("incomplete"), "{error}");
        let receipt = incomplete_receipt
            .take()
            .expect("incomplete process output still leaves a receipt");
        let evidence = receipt.process_evidence.unwrap();
        assert_eq!(
            evidence.launch_state,
            crate::jobs::supervisor::ProcessLaunchState::Exited
        );
        assert_eq!(evidence.output_complete, Some(false));
        assert!(receipt.stdout_hash.is_some() && receipt.stderr_hash.is_some());
        assert!(start.elapsed() < Duration::from_secs(5));
        let port: u16 = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        // Group termination requests precede helper reaping, but the kernel may
        // close a descendant's socket just after it reaps the helper. Require
        // bounded cleanup, well before the fixture's ten-second self-exit.
        let cleanup_deadline = Instant::now() + Duration::from_secs(2);
        while std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(100),
        )
        .is_ok()
        {
            assert!(
                Instant::now() < cleanup_deadline,
                "descendant cleanup timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        for cancel in [false, true] {
            let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let context = ToolContext {
                abort: Some(abort.clone()),
                ..Default::default()
            };
            let trigger = cancel.then(|| {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    abort.store(true, std::sync::atomic::Ordering::SeqCst);
                })
            });
            let start = std::time::Instant::now();
            let error = run(
                &host,
                config("setInterval(()=>{},1000)"),
                b"",
                Some(500),
                Some("0.5"),
                &context,
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if cancel { "aborted" } else { "timed out" }),
                "{error}"
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            if let Some(trigger) = trigger {
                trigger.join().unwrap();
            }
        }
    }
}

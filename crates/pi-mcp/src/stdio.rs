use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use crate::jsonrpc::{Notification, Request, Response};
use crate::{Error, Result, Rpc, CALL_TIMEOUT_SECS};

pub struct StdioTransport {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioTransport {
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<(Self, Child)> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|err| Error::Protocol(format!("spawn `{command}`: {err}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("stdio server has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("stdio server has no stdout".into()))?;
        if let Some(mut stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while stderr.read(&mut buf).ok().filter(|n| *n > 0).is_some() {}
            });
        }
        Ok((
            Self {
                stdin,
                stdout: BufReader::new(stdout),
                next_id: 1,
            },
            child,
        ))
    }

    fn write_line(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, value)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self, id: &Value) -> Result<Value> {
        let deadline = Instant::now() + std::time::Duration::from_secs(CALL_TIMEOUT_SECS);
        let mut line = String::new();
        loop {
            if Instant::now() > deadline {
                return Err(Error::Protocol("mcp call timed out".into()));
            }
            line.clear();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                return Err(Error::Protocol("mcp server closed stdout".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed: Response = serde_json::from_str(trimmed)
                .map_err(|err| Error::Protocol(format!("decode `{trimmed}`: {err}")))?;
            if parsed.id != *id {
                continue;
            }
            if let Some(error) = parsed.error {
                return Err(Error::Protocol(format!(
                    "mcp {}: {}",
                    error.code, error.message
                )));
            }
            return Ok(parsed.result.unwrap_or(Value::Null));
        }
    }
}

impl Rpc for StdioTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request::new(id, method, params);
        let value = serde_json::to_value(&request)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        self.write_line(&value)?;
        self.read_response(&Value::from(id))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let note = Notification::new(method, params);
        let value =
            serde_json::to_value(&note).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        self.write_line(&value)
    }
}

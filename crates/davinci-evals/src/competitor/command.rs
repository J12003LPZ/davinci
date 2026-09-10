//! Generic isolated command runner for external harnesses.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ExternalTask {
    pub repo_path: PathBuf,
    pub request: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalRun {
    pub exit_code: i32,
    pub wall_ms: u64,
    pub files_changed: Vec<String>,
    pub transcript_path: PathBuf,
    pub metrics: BTreeMap<String, f64>,
}

pub trait ExternalHarness: Send + Sync {
    fn name(&self) -> &str;
    fn available(&self) -> Result<bool, String>;
    fn run(&self, task: &ExternalTask) -> Result<ExternalRun, String>;
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

pub fn list_relative_files_with_content(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let walker = walkdir(dir);
    for path in walker {
        if let Ok(rel) = path.strip_prefix(dir) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if let Ok(content) = fs::read(&path) {
                files.insert(rel_str, content);
            }
        }
    }
    files
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                results.extend(walkdir(&p));
            } else if p.is_file() {
                results.push(p);
            }
        }
    }
    results
}

#[derive(Debug, Clone)]
pub struct CommandHarness {
    pub harness_name: String,
    pub binary: PathBuf,
    pub extra_args: Vec<String>,
}

impl CommandHarness {
    pub fn new(
        name: impl Into<String>,
        binary: impl Into<PathBuf>,
        extra_args: Vec<String>,
    ) -> Self {
        Self {
            harness_name: name.into(),
            binary: binary.into(),
            extra_args,
        }
    }
}

impl ExternalHarness for CommandHarness {
    fn name(&self) -> &str {
        &self.harness_name
    }

    fn available(&self) -> Result<bool, String> {
        if self.binary.exists() {
            return Ok(true);
        }
        // Try running --version
        let check = Command::new(&self.binary)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(check.map(|s| s.success()).unwrap_or(false))
    }

    fn run(&self, task: &ExternalTask) -> Result<ExternalRun, String> {
        // 1. Isolate into temporary workspace directory
        let temp_dir =
            tempfile::tempdir().map_err(|e| format!("Failed to create temp sandbox: {e}"))?;
        let isolated_repo = temp_dir.path().join("workspace");
        copy_dir_all(&task.repo_path, &isolated_repo)
            .map_err(|e| format!("Failed to copy repo to sandbox: {e}"))?;

        // 2. Snapshot file state
        let before_files = list_relative_files_with_content(&isolated_repo);

        // 3. Build command
        let mut cmd = Command::new(&self.binary);
        cmd.current_dir(&isolated_repo);
        for arg in &self.extra_args {
            cmd.arg(arg);
        }
        cmd.arg(&task.request);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn {}: {e}", self.binary.display()))?;

        // 4. Wait with timeout
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() > task.timeout {
                        let _ = child.kill();
                        return Err(format!("Task execution timed out after {:?}", task.timeout));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("Error waiting for command: {e}")),
            }
        };

        let wall_ms = start.elapsed().as_millis() as u64;
        let exit_code = status.code().unwrap_or(-1);

        // 5. Read output and save transcript
        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to read command output: {e}"))?;
        let transcript_path = temp_dir.path().join("transcript.log");
        let mut full_output = Vec::new();
        full_output.extend_from_slice(&output.stdout);
        full_output.extend_from_slice(&output.stderr);
        fs::write(&transcript_path, full_output)
            .map_err(|e| format!("Failed to write transcript: {e}"))?;

        // 6. Diff files
        let after_files = list_relative_files_with_content(&isolated_repo);
        let mut changed_paths = BTreeSet::new();

        for (rel, content) in &after_files {
            match before_files.get(rel) {
                Some(prev) if prev == content => {}
                _ => {
                    changed_paths.insert(rel.clone());
                }
            }
        }
        for rel in before_files.keys() {
            if !after_files.contains_key(rel) {
                changed_paths.insert(rel.clone());
            }
        }

        let mut metrics = BTreeMap::new();
        metrics.insert("wall_ms".into(), wall_ms as f64);
        metrics.insert("exit_code".into(), exit_code as f64);
        metrics.insert("files_changed".into(), changed_paths.len() as f64);

        Ok(ExternalRun {
            exit_code,
            wall_ms,
            files_changed: changed_paths.into_iter().collect(),
            transcript_path,
            metrics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_command_runner_captures_changes_and_status() {
        let repo = tempfile::tempdir().unwrap();
        let initial_file = repo.path().join("hello.txt");
        fs::write(&initial_file, "original").unwrap();

        // Create a fake runner executable using python or powershell
        let script = repo.path().join("fake_runner.py");
        fs::write(
            &script,
            "import sys\nwith open('new_file.txt', 'w') as f:\n    f.write('created')\nwith open('hello.txt', 'w') as f:\n    f.write('modified')\nprint('done')\nsys.exit(0)\n"
        ).unwrap();

        let harness =
            CommandHarness::new("fake", "python", vec![script.to_string_lossy().to_string()]);
        assert!(harness.available().unwrap());

        let task = ExternalTask {
            repo_path: repo.path().to_path_buf(),
            request: "do work".into(),
            timeout: Duration::from_secs(10),
        };

        let run = harness.run(&task).unwrap();
        assert_eq!(run.exit_code, 0);
        assert!(run.files_changed.contains(&"hello.txt".to_string()));
        assert!(run.files_changed.contains(&"new_file.txt".to_string()));

        // Original repo was NOT modified (isolation guarantee!)
        assert_eq!(fs::read_to_string(&initial_file).unwrap(), "original");
        assert!(!repo.path().join("new_file.txt").exists());
    }

    #[test]
    fn isolated_command_runner_handles_timeout() {
        let repo = tempfile::tempdir().unwrap();
        let script = repo.path().join("slow_runner.py");
        fs::write(&script, "import time\ntime.sleep(5)\n").unwrap();

        let harness =
            CommandHarness::new("slow", "python", vec![script.to_string_lossy().to_string()]);

        let task = ExternalTask {
            repo_path: repo.path().to_path_buf(),
            request: "slow".into(),
            timeout: Duration::from_millis(300),
        };

        let err = harness.run(&task).unwrap_err();
        assert!(err.contains("timed out"));
    }
}

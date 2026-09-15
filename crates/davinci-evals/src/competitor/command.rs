//! Generic isolated command runner for external harnesses.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::behavior::{run_verification_commands, VerificationCommand, VerificationResult};

use super::probe::{probe_harness, HarnessCapabilities};

#[derive(Debug, Clone)]
pub struct ExternalTask {
    pub repo_path: PathBuf,
    pub request: String,
    pub timeout: Duration,
    pub artifact_dir: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub ignore_paths: Vec<String>,
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

    fn run_with_verification(
        &self,
        task: &ExternalTask,
        verification_commands: &[VerificationCommand],
    ) -> Result<(ExternalRun, Vec<VerificationResult>), String> {
        if !verification_commands.is_empty() {
            return Err(format!(
                "harness '{}' does not expose its isolated workspace for verification",
                self.name()
            ));
        }
        self.run(task).map(|run| (run, Vec::new()))
    }
}

/// Marker contract for named competitor adapters. The execution and
/// isolation boundary remains `ExternalHarness`; adapters only add an
/// explicit identity and capability probe for a specific external CLI.
pub trait CompetitorRunner: ExternalHarness {}

pub fn resolve_runner_binary(environment_name: &str, default_binary: &str) -> PathBuf {
    std::env::var(environment_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_binary))
}

fn resolve_executable(binary: PathBuf) -> PathBuf {
    if binary.is_absolute() || binary.components().count() > 1 {
        return fs::canonicalize(&binary).unwrap_or(binary);
    }

    let Some(path) = std::env::var_os("PATH") else {
        return binary;
    };
    let mut names = vec![binary.clone()];
    #[cfg(windows)]
    if binary.extension().is_none() {
        let extensions = std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect::<Vec<_>>()
            })
            .filter(|extensions| !extensions.is_empty())
            .unwrap_or_else(|| vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()]);
        names.extend(extensions.into_iter().map(|extension| {
            let mut name = binary.as_os_str().to_os_string();
            name.push(extension);
            PathBuf::from(name)
        }));
    }

    for directory in std::env::split_paths(&path) {
        for name in &names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return fs::canonicalize(&candidate).unwrap_or(candidate);
            }
        }
    }
    binary
}

pub fn probe_supported_runner(
    runner_name: &str,
    binary: &Path,
) -> Result<HarnessCapabilities, String> {
    let capabilities = probe_harness(binary)?;
    if capabilities.version.is_none() {
        return Err(format!(
            "competitor runner '{runner_name}' is unsupported: {} did not report a version",
            binary.display()
        ));
    }
    Ok(capabilities)
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    copy_dir_all_ignoring(src, dst, &[])
}

pub fn copy_dir_all_ignoring(
    src: &Path,
    dst: &Path,
    ignore_paths: &[String],
) -> std::io::Result<()> {
    copy_dir_all_impl(src, dst, Path::new(""), ignore_paths)
}

fn copy_dir_all_impl(
    src: &Path,
    dst: &Path,
    relative: &Path,
    ignore_paths: &[String],
) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let entry_relative = relative.join(entry.file_name());
        if should_ignore(&entry_relative, ignore_paths) || ty.is_symlink() {
            continue;
        }
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all_impl(&entry.path(), &dest_path, &entry_relative, ignore_paths)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

pub fn list_relative_files_with_content(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    list_relative_files_with_content_ignoring(dir, &[])
}

pub fn list_relative_files_with_content_ignoring(
    dir: &Path,
    ignore_paths: &[String],
) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(dir, Path::new(""), ignore_paths, &mut files);
    files
}

fn collect_files(
    dir: &Path,
    relative: &Path,
    ignore_paths: &[String],
    files: &mut BTreeMap<String, Vec<u8>>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let entry_relative = relative.join(entry.file_name());
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if should_ignore(&entry_relative, ignore_paths) || file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                collect_files(&path, &entry_relative, ignore_paths, files);
            } else if file_type.is_file() {
                let rel_str = entry_relative.to_string_lossy().replace('\\', "/");
                if let Ok(content) = fs::read(path) {
                    files.insert(rel_str, content);
                }
            }
        }
    }
}

fn should_ignore(relative: &Path, ignore_paths: &[String]) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').collect();
    ignore_paths.iter().any(|ignore| {
        let normalized_ignore = ignore.trim().trim_matches('/').replace('\\', "/");
        !normalized_ignore.is_empty()
            && (normalized == normalized_ignore
                || normalized.starts_with(&format!("{normalized_ignore}/"))
                || components
                    .iter()
                    .any(|component| *component == normalized_ignore))
    })
}

fn default_ignore_paths(task: &ExternalTask) -> Vec<String> {
    let mut ignores = [".git", "target", "node_modules", ".DS_Store", "coverage"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    if let Ok(relative) = task.artifact_dir.strip_prefix(&task.repo_path) {
        if !relative.as_os_str().is_empty() {
            ignores.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    ignores.extend(task.ignore_paths.iter().cloned());
    ignores
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
            binary: resolve_executable(binary.into()),
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
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(check.map(|s| s.success()).unwrap_or(false))
    }

    fn run(&self, task: &ExternalTask) -> Result<ExternalRun, String> {
        self.run_with_verification(task, &[]).map(|(run, _)| run)
    }

    fn run_with_verification(
        &self,
        task: &ExternalTask,
        verification_commands: &[VerificationCommand],
    ) -> Result<(ExternalRun, Vec<VerificationResult>), String> {
        // 1. Isolate into temporary workspace directory
        let temp_dir =
            tempfile::tempdir().map_err(|e| format!("Failed to create temp sandbox: {e}"))?;
        let isolated_repo = temp_dir.path().join("workspace");
        let ignore_paths = default_ignore_paths(task);
        copy_dir_all_ignoring(&task.repo_path, &isolated_repo, &ignore_paths)
            .map_err(|e| format!("Failed to copy repo to sandbox: {e}"))?;

        // 2. Snapshot file state
        let before_files = list_relative_files_with_content_ignoring(&isolated_repo, &ignore_paths);

        // 3. Build command
        let mut cmd = Command::new(&self.binary);
        cmd.current_dir(&isolated_repo);
        for arg in &self.extra_args {
            cmd.arg(arg);
        }
        cmd.arg(&task.request);
        // External harnesses must receive only the explicitly allowlisted
        // environment. In particular, never inherit provider credentials or
        // unrelated host configuration from the evaluator process.
        cmd.env_clear();
        cmd.envs(&task.environment);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let start = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn {}: {e}", self.binary.display()))?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_process_tree(&mut child);
                return Err("Failed to capture external harness stdout".into());
            }
        };
        let stdout_thread = std::thread::spawn(move || read_pipe(stdout));
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_process_tree(&mut child);
                let _ = child.wait();
                let _ = stdout_thread.join();
                return Err("Failed to capture external harness stderr".into());
            }
        };
        let stderr_thread = std::thread::spawn(move || read_pipe(stderr));

        // 4. Wait with timeout
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() > task.timeout {
                        terminate_process_tree(&mut child);
                        let _ = child.wait();
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        return Err(format!("Task execution timed out after {:?}", task.timeout));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    terminate_process_tree(&mut child);
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(format!("Error waiting for command: {e}"));
                }
            }
        };

        let wall_ms = start.elapsed().as_millis() as u64;
        let exit_code = status.code().unwrap_or(-1);

        // 5. Read output and save transcript
        let stdout = stdout_thread
            .join()
            .map_err(|_| "Failed to join stdout reader".to_string())??;
        let stderr = stderr_thread
            .join()
            .map_err(|_| "Failed to join stderr reader".to_string())??;
        fs::create_dir_all(&task.artifact_dir)
            .map_err(|e| format!("Failed to create artifact directory: {e}"))?;
        let transcript_path = task.artifact_dir.join("transcript.log");
        let mut full_output = Vec::new();
        full_output.extend_from_slice(&stdout);
        full_output.extend_from_slice(&stderr);
        fs::write(&transcript_path, full_output)
            .map_err(|e| format!("Failed to write transcript: {e}"))?;

        // 6. Diff files
        let after_files = list_relative_files_with_content_ignoring(&isolated_repo, &ignore_paths);
        let verification = run_verification_commands(verification_commands, &isolated_repo)?;
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

        Ok((
            ExternalRun {
                exit_code,
                wall_ms,
                files_changed: changed_paths.into_iter().collect(),
                transcript_path,
                metrics,
            },
            verification,
        ))
    }
}

fn read_pipe<R: Read>(mut reader: R) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to drain process pipe: {error}"))?;
    Ok(bytes)
}

fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
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
            "import sys\nwith open('new_file.txt', 'w') as f:\n    f.write('created')\nwith open('hello.txt', 'w') as f:\n    f.write('modified')\nwith open('ignored.txt', 'w') as f:\n    f.write('ignored')\nprint('done')\nprint('stderr', file=sys.stderr)\nsys.exit(0)\n"
        ).unwrap();

        let harness =
            CommandHarness::new("fake", "python", vec![script.to_string_lossy().to_string()]);
        assert!(harness.available().unwrap());

        let task = ExternalTask {
            repo_path: repo.path().to_path_buf(),
            request: "do work".into(),
            timeout: Duration::from_secs(10),
            artifact_dir: repo.path().join("artifacts"),
            environment: BTreeMap::new(),
            ignore_paths: vec!["ignored.txt".into()],
        };

        let run = harness.run(&task).unwrap();
        assert_eq!(run.exit_code, 0);
        assert!(run.files_changed.contains(&"hello.txt".to_string()));
        assert!(run.files_changed.contains(&"new_file.txt".to_string()));
        assert!(!run.files_changed.contains(&"ignored.txt".to_string()));

        // Original repo was NOT modified (isolation guarantee!)
        assert_eq!(fs::read_to_string(&initial_file).unwrap(), "original");
        assert!(!repo.path().join("new_file.txt").exists());
        assert!(run.transcript_path.is_file());
        let transcript = fs::read_to_string(run.transcript_path).unwrap();
        assert!(transcript.contains("done"));
        assert!(transcript.contains("stderr"));
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
            artifact_dir: repo.path().join("artifacts"),
            environment: BTreeMap::new(),
            ignore_paths: Vec::new(),
        };

        let err = harness.run(&task).unwrap_err();
        assert!(err.contains("timed out"));
    }
}

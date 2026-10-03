//! Bounded, non-interactive Git execution for graph bookkeeping.
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_OUTPUT_CAP: usize = 32 * 1024 * 1024;

pub fn run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(davinci_sys::process::resolve_program("git"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .current_dir(cwd)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.pager=cat",
        ])
        .args(args);
    let output = davinci_sys::process::run_bounded(
        command,
        None,
        davinci_sys::process::RunLimits {
            timeout: GIT_TIMEOUT,
            output_cap: GIT_OUTPUT_CAP,
        },
        &|| false,
    )
    .map_err(|error| format!("git {} failed to start: {error}", args.join(" ")))?;
    if output.timed_out {
        return Err(format!("git {} timed out after 30s", args.join(" ")));
    }
    let status = output
        .status
        .ok_or_else(|| format!("git {} did not return a status", args.join(" ")))?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git {} failed ({}): {}",
            args.join(" "),
            status,
            stderr.trim()
        ));
    }
    Ok(output.stdout)
}

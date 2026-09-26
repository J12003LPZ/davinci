//! Conservative verification classification; never grants execution authority.
use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Suite,
    TargetedScript,
    SyntaxOnly,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Assessment {
    pub kind: CheckKind,
    pub covered: Vec<PathBuf>,
    pub complete: bool,
    pub reason: &'static str,
}

impl Assessment {
    fn unknown(reason: &'static str) -> Self {
        Self {
            kind: CheckKind::Unknown,
            covered: Vec::new(),
            complete: false,
            reason,
        }
    }
}

#[derive(Clone, serde::Deserialize)]
struct SyntaxFacts {
    checked_modules: Vec<String>,
    checked_files: Vec<String>,
    imports: Vec<String>,
}
type InterpreterIdentity = (PathBuf, u64, std::time::SystemTime);
type SyntaxCache = VecDeque<(InterpreterIdentity, String, SyntaxFacts)>;
static CACHE: OnceLock<Mutex<SyntaxCache>> = OnceLock::new();

fn interpreter(name: &str) -> Option<PathBuf> {
    // Never resolve against the inspected workspace or execute a shell wrapper.
    // The host PATH is the same authority used by the already executed command.
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    if candidate.components().count() != 1 {
        return None;
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        if !directory.is_absolute() {
            continue;
        }
        for suffix in if cfg!(windows) {
            &[".exe", ""][..]
        } else {
            &[""][..]
        } {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn syntax(name: &str, source: &str) -> Option<SyntaxFacts> {
    if source.len() > 65536 {
        return None;
    }
    let interpreter = interpreter(name)?;
    let metadata = std::fs::metadata(&interpreter).ok()?;
    let identity = (
        interpreter.clone(),
        metadata.len(),
        metadata.modified().ok()?,
    );
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Some((_, _, facts)) = cache
        .lock()
        .ok()?
        .iter()
        .find(|(p, s, _)| p == &identity && s == source)
    {
        return Some(facts.clone());
    }
    let mut command = Command::new(&interpreter);
    command
        .args(["-I", "-S", "-c", include_str!("verification/python_ast.py")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let started = Instant::now();
    let mut child = command.spawn().ok()?;
    let mut input = child.stdin.take()?;
    let output = child.stdout.take()?;
    let request = serde_json::to_vec(&serde_json::json!({"source": source})).ok()?;
    let writer = std::thread::spawn(move || input.write_all(&request));
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.take(16385).read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < Duration::from_millis(250) => {
                std::thread::sleep(Duration::from_millis(2))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let written = writer.join().ok().and_then(Result::ok);
    let bytes = reader.join().ok().and_then(Result::ok)?;
    if !status.is_some_and(|s| s.success()) || written.is_none() || bytes.len() > 16384 {
        return None;
    }
    let facts: SyntaxFacts = serde_json::from_slice(&bytes).ok()?;
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back((identity, source.into(), facts.clone()));
    Some(facts)
}

/// Split only literal AND chains. Other unquoted shell control is inconclusive.
fn segments(command: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut text = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            text.push(ch);
            if ch == q {
                quote = None;
            } else if q == '"' && matches!(ch, '$' | '`' | '\\') {
                return None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                text.push(ch);
            }
            '&' if chars.next_if_eq(&'&').is_some() => {
                if text.trim().is_empty() {
                    return None;
                }
                result.push(std::mem::take(&mut text));
            }
            '&' | '|' | ';' | '\n' | '\r' | '$' | '`' | '<' | '>' | '(' | ')' => return None,
            _ => text.push(ch),
        }
    }
    if quote.is_some() || text.trim().is_empty() {
        return None;
    }
    result.push(text);
    Some(result)
}

fn normalized(root: &Path, path: &Path) -> Option<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut parts = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                if !parts.pop() {
                    return None;
                }
            }
            std::path::Component::CurDir => {}
            _ => parts.push(part.as_os_str()),
        }
    }
    let identity = std::fs::canonicalize(&parts).unwrap_or(parts);
    Some(crate::permission::strip_verbatim_prefix(&identity))
}

fn module_paths(root: &Path, module: &str) -> Vec<PathBuf> {
    if !module
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Vec::new();
    }
    let base = module.replace('.', "/");
    [format!("{base}.py"), format!("{base}/__init__.py")]
        .iter()
        .map(|name| root.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn exempt(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    matches!(
        path.extension().and_then(|v| v.to_str()),
        Some("md" | "rst" | "txt")
    ) || name.starts_with("test_")
        || name.ends_with("_test.py")
        || path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some("tests" | "fixtures" | "docs")
            )
        })
}

pub fn classify(tool: &str, command: &str, cwd: &Path, paths: &[PathBuf]) -> Assessment {
    if command.len() > 65536 {
        return Assessment::unknown("source_limit");
    }
    let posix = tool == "bash" || (tool == "exec_command" && !cfg!(windows));
    let command = if posix {
        command.trim().strip_prefix("set -eu\n").unwrap_or(command.trim())
    } else { command.trim() };
    let mut source = None;
    let mut header = command;
    if let Some((head, body)) = command.split_once('\n') {
        if !posix {
            return Assessment::unknown("unsupported_shell");
        }
        let Some((prefix, marker)) = head.rsplit_once("<<") else {
            return Assessment::unknown("shell_control");
        };
        let marker = marker.trim();
        let Some(marker) = marker.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) else {
            return Assessment::unknown("unquoted_heredoc");
        };
        if marker.is_empty()
            || !marker
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Assessment::unknown("heredoc_marker");
        }
        let Some(body) = body.strip_suffix(marker).and_then(|s| s.strip_suffix('\n')) else {
            return Assessment::unknown("heredoc_tail");
        };
        if body.lines().any(|line| line == marker) {
            return Assessment::unknown("heredoc_tail");
        }
        source = Some(body);
        header = prefix;
    }
    let Some(parts) = segments(header) else {
        return Assessment::unknown("masked_status");
    };
    let mut directory = cwd.to_path_buf();
    let mut covered = BTreeSet::new();
    let mut kind = CheckKind::Unknown;
    for (index, part) in parts.iter().enumerate() {
        let Some(words) = crate::shell_policy::literal_shell_words(part.trim()) else {
            return Assessment::unknown("nonliteral_command");
        };
        if words.is_empty() { return Assessment::unknown("empty_command"); }
        if words.len() == 2 && words[0] == "cd" && index == 0 {
            let Some(path) = normalized(cwd, Path::new(&words[1])) else {
                return Assessment::unknown("cwd");
            };
            directory = path;
            continue;
        }
        if words.iter().any(|s| {
            matches!(
                s.as_str(),
                "--help"
                    | "-h"
                    | "--version"
                    | "--no-run"
                    | "--collect-only"
                    | "--co"
                    | "--list"
                    | "--fixtures"
                    | "--fixtures-per-test"
                    | "--setup-plan"
                    | "-V"
            )
        }) {
            return Assessment::unknown("noop_option");
        }
        let python = matches!(
            words[0].as_str(),
            "python" | "python3" | "python.exe" | "python3.exe"
        );
        if python && std::env::var_os("PYTHONOPTIMIZE").is_some_and(|v| !v.is_empty() && v != "0") {
            return Assessment::unknown("assertions_disabled");
        }
        let inline = if python && words.len() == 3 && words[1] == "-c" {
            Some(words[2].as_str())
        } else if python && words.len() == 2 && words[1] == "-" && index + 1 == parts.len() {
            source
        } else {
            None
        };
        if let Some(inline) = inline {
            let Some(facts) = syntax(&words[0], inline) else {
                return Assessment::unknown("inspection_unavailable");
            };
            if facts.checked_modules.is_empty() && facts.checked_files.is_empty() {
                return Assessment::unknown("no_applicable_check");
            }
            let mut targets = Vec::new();
            for module in facts.checked_modules {
                targets.extend(module_paths(&directory, &module));
            }
            targets.extend(facts.checked_files.into_iter().map(|p| directory.join(p)));
            // Bounded, syntax-only local dependencies of directly checked changed modules.
            for target in targets.clone().into_iter().take(32) {
                if !paths
                    .iter()
                    .any(|path| normalized(cwd, path) == normalized(cwd, &target))
                {
                    continue;
                }
                let Ok(file) = std::fs::File::open(&target) else {
                    continue;
                };
                let mut source = String::new();
                if file.take(65537).read_to_string(&mut source).is_err() || source.len() > 65536 {
                    continue;
                }
                if let Some(facts) = syntax(&words[0], &source) {
                    for module in facts.imports {
                        targets.extend(module_paths(&directory, &module));
                    }
                }
            }
            for path in paths {
                if targets
                    .iter()
                    .any(|target| normalized(&directory, target) == normalized(cwd, path))
                {
                    covered.insert(path.clone());
                }
            }
            if kind != CheckKind::Suite {
                kind = CheckKind::TargetedScript;
            }
        } else if python
            && words.len() > 3
            && words[1] == "-m"
            && matches!(words[2].as_str(), "py_compile" | "compileall")
        {
            if kind == CheckKind::Unknown {
                kind = CheckKind::SyntaxOnly;
            }
        } else if crate::shell_policy::verification_outcome(part.trim()) == Some(true) {
            for path in paths {
                if let Some(relative) = normalized(cwd, path)
                    .and_then(|path| path.strip_prefix(&directory).ok().map(Path::to_path_buf))
                {
                    if matches!(
                        crate::verification_coverage_for_command(part.trim(), &[relative]).0,
                        crate::VerificationCoverage::Broad | crate::VerificationCoverage::Targeted
                    ) {
                        covered.insert(path.clone());
                    }
                }
            }
            kind = CheckKind::Suite;
        } else {
            return Assessment::unknown("unsupported_command");
        }
    }
    if !covered.is_empty() {
        covered.extend(paths.iter().filter(|path| exempt(path)).cloned());
    }
    let complete = !paths.is_empty() && covered.len() == paths.len();
    Assessment {
        kind,
        covered: covered.into_iter().collect(),
        complete,
        reason: "classified",
    }
}

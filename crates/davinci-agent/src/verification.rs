//! Conservative verification classification; never grants execution authority.
pub mod acceptance;
pub(crate) mod discovery;
pub(crate) mod workspace;

/// Whether a command invokes a recognized test runner. Recognition does not
/// imply that its output format is supported or that any tests ran.
pub fn is_test_runner(command: &str) -> bool {
    discovery::runner(command).is_some()
}

/// Parse discovery evidence from the original captured process streams.
pub fn test_discovery(
    command: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Option<crate::runtime::evidence::AssertionCounts> {
    discovery::counts(command, stdout, stderr)
}

pub fn tests_passed(counts: &crate::runtime::evidence::AssertionCounts) -> bool {
    counts.passed > 0
        && counts.failed == 0
        && counts.passed.checked_add(counts.skipped) == Some(counts.total)
}
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
    /// An unfiltered suite at the workspace root can cover unnamed mutations.
    pub full_workspace: bool,
    pub reason: &'static str,
}

impl Assessment {
    fn unknown(reason: &'static str) -> Self {
        Self {
            kind: CheckKind::Unknown,
            covered: Vec::new(),
            complete: false,
            full_workspace: false,
            reason,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
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
    command.args(["-I", "-S", "-c", include_str!("verification/python_ast.py")]);
    // Classification is deterministic; loaded CI must not turn a coverage
    // regression into a process-start timing test. Production remains bounded.
    #[cfg(test)]
    let inspection_budget = Duration::from_secs(10);
    #[cfg(all(windows, not(test)))]
    let inspection_budget = Duration::from_secs(1);
    #[cfg(all(not(windows), not(test)))]
    let inspection_budget = Duration::from_millis(250);
    let facts = inspect_command(command, source, inspection_budget)?;
    let mut cache = cache.lock().ok()?;
    cache_insert(&mut cache, identity, source.to_string(), facts.clone());
    Some(facts)
}

fn cache_insert(
    cache: &mut SyntaxCache,
    identity: InterpreterIdentity,
    source: String,
    facts: SyntaxFacts,
) {
    cache.retain(|(key, text, _)| key != &identity || text != &source);
    if cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back((identity, source, facts));
}

fn inspect_command(mut command: Command, source: &str, budget: Duration) -> Option<SyntaxFacts> {
    command
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
            Ok(None) if started.elapsed() < budget => std::thread::sleep(Duration::from_millis(2)),
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
    serde_json::from_slice(&bytes).ok()
}

/// Split only literal AND chains. Other unquoted shell control is inconclusive.
fn segments(command: &str, posix: bool) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut text = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            text.push(ch);
            if ch == q {
                if !posix && chars.next_if_eq(&q).is_some() {
                    text.push(q);
                } else {
                    quote = None;
                }
            } else if q == '"' && (matches!(ch, '$' | '`') || (posix && ch == '\\')) {
                return None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                text.push(ch);
            }
            '&' if posix && chars.next_if_eq(&'&').is_some() => {
                if text.trim().is_empty() {
                    return None;
                }
                result.push(std::mem::take(&mut text));
            }
            '&' | '|' | ';' | '\n' | '\r' | '$' | '`' | '<' | '>' | '(' | ')' | '#' => return None,
            _ => text.push(ch),
        }
    }
    if quote.is_some() || text.trim().is_empty() {
        return None;
    }
    result.push(text);
    Some(result)
}

fn literal_words(command: &str, posix: bool) -> Option<Vec<String>> {
    if posix {
        return crate::shell_policy::literal_shell_words(command);
    }
    // PowerShell uses backticks for escaping, doubled quotes inside quoted
    // strings, and literal backslashes in Windows paths. Its 5.1 grammar does
    // not support &&; segments deliberately rejects all PS command chains.
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                if chars.next_if_eq(&q).is_some() {
                    word.push(q);
                } else {
                    quote = None;
                }
            } else if q == '"' && matches!(ch, '$' | '`') {
                return None;
            } else {
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>' | ';' | '|' | '&' | '@' | '#' => {
                return None
            }
            ch if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

pub(crate) fn normalized(root: &Path, path: &Path) -> Option<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    // Deleted/new paths still inherit the identity of their nearest existing
    // ancestor. This also resolves symlinked cwd and Windows short/case aliases.
    // Resolve before removing `..`: a symlink's parent can differ from the
    // lexical parent of the link itself.
    let mut ancestor = path.as_path();
    let mut suffix = Vec::new();
    let identity = loop {
        if let Ok(mut identity) = std::fs::canonicalize(ancestor) {
            for name in suffix.into_iter().rev() {
                identity.push(name);
            }
            break identity;
        }
        suffix.push(ancestor.file_name()?.to_os_string());
        ancestor = ancestor.parent()?;
    };
    let identity = crate::permission::strip_verbatim_prefix(&identity);
    #[cfg(windows)]
    let identity = PathBuf::from(identity.to_string_lossy().to_lowercase());
    Some(identity)
}

fn module_paths(root: &Path, module: &str) -> Vec<PathBuf> {
    if !module
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Vec::new();
    }
    let mut directory = root.to_path_buf();
    let mut parts = module.split('.').peekable();
    while let Some(part) = parts.next() {
        let package = directory.join(part);
        let init = package.join("__init__.py");
        let source = directory.join(format!("{part}.py"));
        // FileFinder chooses regular packages before same-named source files.
        // A source module also shadows a namespace directory and cannot have a
        // submodule. Do not credit both sides of such a basename collision.
        if init.is_file() {
            if parts.peek().is_none() {
                return vec![init];
            }
        } else if source.is_file() {
            return if parts.peek().is_none() {
                vec![source]
            } else {
                Vec::new()
            };
        } else if !package.is_dir() {
            return Vec::new();
        }
        directory = package;
    }
    Vec::new()
}

fn exempt(workspace: &Path, path: &Path) -> bool {
    normalized(workspace, path)
        .and_then(|path| {
            path.strip_prefix(normalized(workspace, Path::new("."))?)
                .ok()
                .map(Path::to_path_buf)
        })
        .is_some_and(|relative| {
            matches!(
                relative.extension().and_then(|v| v.to_str()),
                Some("md" | "rst")
            ) || (relative.starts_with("docs") && relative.extension().is_some_and(|v| v == "txt"))
        })
}

pub fn classify(tool: &str, command: &str, cwd: &Path, paths: &[PathBuf]) -> Assessment {
    classify_in_workspace(tool, command, cwd, cwd, paths)
}

fn python_name(name: &str) -> bool {
    let name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(name);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let Some(version) = name.strip_prefix("python") else {
        return false;
    };
    version.is_empty()
        || version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn unfiltered_suite(words: &[String]) -> bool {
    let args = match words {
        [python, module, runner, discover, rest @ ..]
            if python_name(python)
                && module == "-m"
                && runner == "unittest"
                && discover == "discover" =>
        {
            return rest
                .iter()
                .all(|arg| matches!(arg.as_str(), "-q" | "--quiet" | "-v" | "--verbose"));
        }
        [python, module, runner, rest @ ..]
            if python_name(python) && module == "-m" && runner == "pytest" =>
        {
            rest
        }
        [runner, rest @ ..] if runner == "pytest" || runner == "pytest.exe" => rest,
        [runner, action, rest @ ..]
            if runner == "cargo" && matches!(action.as_str(), "test" | "check" | "clippy") =>
        {
            rest
        }
        [runner, action, rest @ ..]
            if matches!(runner.as_str(), "npm" | "pnpm" | "yarn") && action == "test" =>
        {
            rest
        }
        [runner, action, target, rest @ ..]
            if runner == "go" && action == "test" && target == "./..." =>
        {
            rest
        }
        _ => return false,
    };
    args.iter().all(|arg| {
        matches!(
            arg.as_str(),
            "-q" | "--quiet"
                | "-v"
                | "-vv"
                | "--verbose"
                | "--offline"
                | "--locked"
                | "--workspace"
                | "--all-targets"
                | "--all-features"
                | "--release"
        )
    })
}

/// Plain-language reason a post-edit command is not credited as a check.
pub(crate) fn unrecognized_check_reason(assessment: &Assessment) -> &'static str {
    if assessment.kind == CheckKind::SyntaxOnly {
        return "a syntax check does not verify behavior";
    }
    match assessment.reason {
        "nonliteral_command" => "shell variables, `$(...)` and other expansions are not inspected",
        "unsupported_shell" => "PowerShell here-strings and heredocs are not inspected",
        "masked_status" => "its exit status comes from a later command in a `;`, `|` or `||` chain",
        "no_applicable_check" => {
            "it asserts nothing about the changed code; printing a result is not a check"
        }
        "unsupported_command" => "running a program directly is not recognized as a check",
        "unquoted_heredoc" => "only a quoted heredoc such as `<<'PY'` is inspected",
        "heredoc_marker" | "heredoc_tail" => "the heredoc could not be parsed",
        "inspection_unavailable" => "the check's source could not be analyzed",
        "noop_option" => "that option does not run any check",
        "assertions_disabled" => "PYTHONOPTIMIZE disables assert statements",
        "cwd" => "its working directory could not be resolved",
        _ => "the harness could not classify it",
    }
}

pub(crate) fn classify_in_workspace(
    tool: &str,
    command: &str,
    cwd: &Path,
    workspace: &Path,
    paths: &[PathBuf],
) -> Assessment {
    if command.len() > 65536 {
        return Assessment::unknown("source_limit");
    }
    let posix = tool == "bash" || (tool == "exec_command" && !cfg!(windows));
    let command = if posix {
        command
            .trim()
            .strip_prefix("set -eu\n")
            .unwrap_or(command.trim())
    } else {
        command.trim()
    };
    let mut source = None;
    let mut header = command;
    if let Some((head, body)) = command
        .split_once('\n')
        .filter(|(head, _)| head.contains("<<"))
    {
        if !posix {
            return Assessment::unknown("unsupported_shell");
        }
        let (prefix, marker) = head.rsplit_once("<<").unwrap();
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
    let Some(parts) = segments(header, posix) else {
        return Assessment::unknown("masked_status");
    };
    let Some(mut directory) = normalized(cwd, Path::new(".")) else {
        return Assessment::unknown("cwd");
    };
    let workspace =
        normalized(workspace, Path::new(".")).unwrap_or_else(|| workspace.to_path_buf());
    let mut covered = BTreeSet::new();
    let mut kind = CheckKind::Unknown;
    let mut full_workspace = false;
    for (index, part) in parts.iter().enumerate() {
        let Some(words) = literal_words(part.trim(), posix) else {
            return Assessment::unknown("nonliteral_command");
        };
        if words.is_empty() {
            return Assessment::unknown("empty_command");
        }
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
        let python = python_name(&words[0]);
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
                    .any(|path| normalized(&workspace, path) == normalized(&directory, &target))
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
                    .any(|target| normalized(&directory, target) == normalized(&workspace, path))
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
        } else if crate::shell_policy::verification_outcome(&{
            let mut suite_words = words.clone();
            if python {
                suite_words[0] = "python".into();
            }
            suite_words.join(" ")
        }) == Some(true)
        {
            let suite = words.join(" ");
            full_workspace |= directory == workspace && unfiltered_suite(&words);
            for path in paths {
                // unittest selectors and discovery patterns do not establish
                // coverage of an entire source tree merely by naming tests/.
                if python
                    && words.get(2).is_some_and(|runner| runner == "unittest")
                    && !unfiltered_suite(&words)
                {
                    continue;
                }
                if let Some(relative) = normalized(&workspace, path)
                    .and_then(|path| path.strip_prefix(&directory).ok().map(Path::to_path_buf))
                {
                    let coverage = crate::verification_coverage_for_command(&suite, &[relative]).0;
                    if coverage == crate::VerificationCoverage::Targeted
                        || (coverage == crate::VerificationCoverage::Broad
                            && unfiltered_suite(&words))
                    {
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
        covered.extend(
            paths
                .iter()
                .filter(|path| exempt(&workspace, path))
                .cloned(),
        );
    }
    let complete = !paths.is_empty() && covered.len() == paths.len();
    Assessment {
        kind,
        covered: covered.into_iter().collect(),
        complete,
        full_workspace,
        reason: "classified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_python_and_powershell_paths_are_literal() {
        for name in ["python", "python3", "python3.11", "python3.13.exe"] {
            assert!(python_name(name));
        }
        for name in ["python-wrapper", "python3.", "python..3", "python3.bad"] {
            assert!(!python_name(name));
        }
        assert_eq!(
            literal_words(r"pytest 'tests\test_file.py'", false).unwrap(),
            ["pytest", "tests\\test_file.py"]
        );
    }

    #[test]
    fn syntax_cache_is_bounded_and_replaces_duplicates() {
        let mut cache = SyntaxCache::new();
        let identity = (PathBuf::from("python"), 1, std::time::UNIX_EPOCH);
        let facts = SyntaxFacts {
            checked_modules: Vec::new(),
            checked_files: Vec::new(),
            imports: Vec::new(),
        };
        for index in 0..256 {
            cache_insert(
                &mut cache,
                identity.clone(),
                index.to_string(),
                facts.clone(),
            );
        }
        assert_eq!(cache.len(), 128);
        assert_eq!(cache.front().unwrap().1, "128");
        cache_insert(&mut cache, identity, "255".into(), facts);
        assert_eq!(cache.len(), 128);
    }

    #[test]
    fn helper_timeout_kills_and_reaps_and_does_not_poison_later_inspection() {
        let python = interpreter("python").expect("Python is required for verification tests");
        let mut command = Command::new(&python);
        command.args(["-I", "-S", "-c", "import time; time.sleep(30)"]);
        let start = Instant::now();
        assert!(inspect_command(command, "assert True", Duration::from_millis(25)).is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(syntax("python", "import changed; assert changed.f(1) == 2").is_some());
    }

    #[test]
    fn helper_response_and_source_bounds_are_enforced() {
        assert!(syntax("python", &"x".repeat(65_537)).is_none());
        let mut command = Command::new(interpreter("python").unwrap());
        command.args(["-I", "-S", "-c", "print('x' * 16385)"]);
        assert!(inspect_command(command, "", Duration::from_millis(250)).is_none());
    }

    #[test]
    fn focused_unittest_does_not_cover_other_changed_modules() {
        let root = tempfile::tempdir().unwrap();
        let paths = [PathBuf::from("app.py"), PathBuf::from("other.py")];
        for command in [
            "python -m unittest test_app.TestApp.test_value",
            "python -m unittest discover -s tests -p test_app.py",
            "python -m unittest discover -k test_value",
        ] {
            let focused = classify_in_workspace("bash", command, root.path(), root.path(), &paths);
            assert!(!focused.complete, "{command}");
            assert!(!focused.full_workspace, "{command}");
        }
        let whole = classify_in_workspace(
            "bash",
            "python -m unittest discover",
            root.path(),
            root.path(),
            &paths,
        );
        assert!(whole.complete);
        assert!(whole.full_workspace);
    }

    #[test]
    fn subdirectory_suite_cannot_clear_workspace_unknown_scope() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let result = classify_in_workspace(
            "bash",
            "pytest -q",
            &sub,
            root.path(),
            &[PathBuf::from("sub/a.py")],
        );
        assert!(result.complete);
        assert!(!result.full_workspace);
    }

    #[test]
    fn module_resolution_does_not_credit_shadowed_source_or_submodules() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pkg")).unwrap();
        std::fs::write(root.path().join("pkg/__init__.py"), "").unwrap();
        std::fs::write(root.path().join("pkg.py"), "").unwrap();
        assert_eq!(
            module_paths(root.path(), "pkg"),
            [root.path().join("pkg/__init__.py")]
        );
        std::fs::remove_file(root.path().join("pkg/__init__.py")).unwrap();
        std::fs::write(root.path().join("pkg/child.py"), "").unwrap();
        assert_eq!(
            module_paths(root.path(), "pkg"),
            [root.path().join("pkg.py")]
        );
        assert!(module_paths(root.path(), "pkg.child").is_empty());
    }
}

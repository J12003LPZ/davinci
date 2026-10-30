//! Discovery from actual captured runner output, never decorated tool text.
use crate::runtime::evidence::AssertionCounts;
use std::sync::OnceLock;

pub(crate) fn runner(command: &str) -> Option<&'static str> {
    let words =
        super::literal_words(command, true).or_else(|| super::literal_words(command, false))?;
    runner_words(&words)
}

pub(crate) fn runner_words(words: &[String]) -> Option<&'static str> {
    // Match shell verification classification, including literal assignments
    // outside or inside RTK's transparent wrapper. Assignment values do not
    // supply runner identity or evidence.
    let start = words
        .iter()
        .position(|word| !crate::shell_policy::shell_env_assignment(word))?;
    let words = &words[start..];
    let (name, rest) = words.split_first()?;
    let name = name.rsplit(['/', '\\']).next()?.trim_end_matches(".exe");
    if name == "rtk" {
        return runner_words(if rest.first().is_some_and(|s| s == "proxy") {
            &rest[1..]
        } else {
            rest
        });
    }
    if name == "cargo" {
        let mut args = rest.iter();
        while let Some(arg) = args.next() {
            if matches!(arg.as_str(), "--config" | "--color" | "-C") {
                args.next()?;
            } else if !(arg.starts_with('-') || arg.starts_with('+')) {
                return (arg == "test").then_some("rust");
            }
        }
        return None;
    }
    match words {
        [name, args @ ..]
            if matches!(name.as_str(), "node" | "node.exe")
                && args.iter().any(|arg| arg == "--test") =>
        {
            Some(if args.iter().any(|arg| arg == "--test-reporter=tap") {
                "node-tap"
            } else {
                "unknown"
            })
        }
        [name, action, ..] if name == "cargo" && action == "test" => Some("rust"),
        [name, module, runner, ..]
            if super::python_name(name) && module == "-m" && runner == "unittest" =>
        {
            Some("unittest")
        }
        [name, module, runner, ..]
            if super::python_name(name) && module == "-m" && runner == "pytest" =>
        {
            Some("pytest")
        }
        [name, ..] if matches!(name.as_str(), "pytest" | "pytest.exe") => Some("pytest"),
        // The script behind a package manager's `test` is not visible here, so
        // only the output it produced can show which runner ran.
        [name, action, ..]
            if matches!(name.as_str(), "npm" | "pnpm" | "yarn") && action == "test" =>
        {
            Some("package-script")
        }
        [name, action, ..] if name == "go" && action == "test" => Some("unknown"),
        _ => None,
    }
}

pub(crate) fn counts(command: &str, stdout: &[u8], stderr: &[u8]) -> Option<AssertionCounts> {
    counts_in(None, command, stdout, stderr)
}

/// [`counts`] for a command whose package was bound before it started. A
/// package-manager `test` script only has an identity in the manifest it ran, so
/// without a [`PackageBinding`] that wrapper yields no evidence at all.
pub(crate) fn counts_in(
    package: Option<&PackageBinding>,
    command: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Option<AssertionCounts> {
    if stdout.len().checked_add(stderr.len())? > 4 * 1024 * 1024 {
        return None;
    }
    let stdout = std::str::from_utf8(stdout).ok()?;
    let stderr = std::str::from_utf8(stderr).ok()?;
    let mut counts = AssertionCounts::default();
    match runner(command)? {
        "node-tap" => {
            // Node prefixes test stdout with an extra '# ', so emitted text
            // cannot masquerade as its own top-level discovery summary.
            let mut values = std::collections::BTreeMap::new();
            for line in stdout.lines() {
                let Some((key, value)) = line.strip_prefix("# ").and_then(|s| s.split_once(' '))
                else {
                    continue;
                };
                if ["tests", "pass", "fail", "cancelled", "skipped", "todo"].contains(&key)
                    && values.insert(key, value.parse::<u32>().ok()?).is_some()
                {
                    return None;
                }
            }
            counts.total = *values.get("tests")?;
            counts.passed = *values.get("pass")?;
            counts.failed = values.get("fail")?.checked_add(*values.get("cancelled")?)?;
            counts.skipped = values.get("skipped")?.checked_add(*values.get("todo")?)?;
            if counts
                .passed
                .checked_add(counts.failed)?
                .checked_add(counts.skipped)?
                != counts.total
            {
                return None;
            }
        }
        "rust" => {
            static SUMMARY: OnceLock<regex::Regex> = OnceLock::new();
            let pattern = SUMMARY.get_or_init(|| regex::Regex::new(
                r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; \d+ filtered out; finished in .+$"
            ).expect("static test summary pattern"));
            let mut found = false;
            for line in stdout.lines().chain(stderr.lines()) {
                let Some(summary) = pattern.captures(line.trim()) else {
                    continue;
                };
                counts.passed = counts.passed.checked_add(summary[1].parse::<u32>().ok()?)?;
                counts.failed = counts.failed.checked_add(summary[2].parse::<u32>().ok()?)?;
                counts.skipped = counts
                    .skipped
                    .checked_add(summary[3].parse::<u32>().ok()?)?;
                found = true;
            }
            if !found {
                return None;
            }
            counts.total = counts
                .passed
                .checked_add(counts.failed)?
                .checked_add(counts.skipped)?;
        }
        "unittest" => {
            // Only the standard runner's final summary; custom runners stay unknown.
            let lines: Vec<_> = stderr
                .lines()
                .chain(stdout.lines())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let end = *lines.last()?;
            counts.skipped = if end == "OK" {
                0
            } else {
                end.strip_prefix("OK (skipped=")?
                    .strip_suffix(')')?
                    .parse()
                    .ok()?
            };
            let summary = lines.iter().rev().nth(1)?.strip_prefix("Ran ")?;
            let (number, rest) = summary.split_once(' ')?;
            if !(rest.starts_with("tests in ") || rest.starts_with("test in ")) {
                return None;
            }
            counts.total = number.parse().ok()?;
            counts.passed = counts.total.checked_sub(counts.skipped)?;
        }
        "package-script" => return package_script_unittest(package?, command, stderr),
        "pytest" => {
            // Only pytest's final terminal summary; earlier lines can be test output.
            let end = stdout.lines().map(str::trim).rfind(|s| !s.is_empty())?;
            return pytest_summary(end);
        }
        _ => return None,
    }
    Some(counts)
}

/// What a package manager's `test` is going to run, read before the process
/// starts: the manifest the package manager will read in the directory it runs
/// in, its digest, and whether its `test` script is exactly the unittest runner.
///
/// The host binds it before dispatch. Reading the manifest after the run would
/// let the script rewrite its own identity (`node fake.js` that stores
/// `python -m unittest` in `scripts.test` and prints a footer).
#[derive(Debug, Clone)]
pub(crate) struct PackageBinding {
    cwd: std::path::PathBuf,
    manifest_sha256: String,
    runs_unittest: bool,
}

impl PackageBinding {
    /// `None` when `cwd` has no readable manifest. A manifest the package manager
    /// would not run as written still binds, as not unittest.
    pub(crate) fn bind(cwd: &std::path::Path) -> Option<Self> {
        let cwd = cwd.canonicalize().ok()?;
        let bytes = read_manifest(&cwd)?;
        Some(Self {
            runs_unittest: script_is_unittest(&cwd, &bytes),
            manifest_sha256: crate::runtime::checkpoints::compute_sha256(&bytes),
            cwd,
        })
    }

    /// The canonical directory the manifest was read in.
    pub(crate) fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// The manifest on disk now is byte for byte the one bound before the run.
    fn unchanged(&self) -> bool {
        read_manifest(&self.cwd).is_some_and(|bytes| {
            crate::runtime::checkpoints::compute_sha256(&bytes) == self.manifest_sha256
        })
    }
}

/// A package-manager `test` script that ran Python's standard unittest runner.
///
/// Output alone cannot say which program wrote it: any script can print a
/// unittest-shaped footer. So these independent things must hold.
///
/// 0. The command is exactly `<npm|pnpm|yarn> test`: no flag or argument that
///    could redirect the package (`--prefix`, `-C`, `--workspace`), no
///    environment assignment, no wrapper.
/// 1. Identity, from the manifest the package manager ran, bound before the
///    process started (see [`PackageBinding`]): its `test` script is exactly
///    the unittest runner (no chaining, no other program), it defines no
///    `pretest` or `posttest`, no package-manager configuration file sits
///    beside it, and the manifest is unchanged after the run.
/// 2. Discovery, from the runner's own progress records: a line of one
///    character per test (`.` passed, `s` skipped) before the rule, or one
///    `name ... ok` line per test in verbose mode. The number of records must
///    equal the `Ran N tests` count, and the skipped records must equal the
///    verdict's `skipped=` count. A footer without the tests it summarizes
///    proves nothing.
/// 3. The footer: a rule of 70 dashes, `Ran N tests in S.SSSs` and the `OK`
///    verdict closing stderr.
///
/// Any other runner, or any mismatch, stays unknown. An exit status other than
/// zero is the receipt's concern: it never reaches a passing check.
///
/// What this still trusts: the script that literally is `python -m unittest`
/// resolves to the real runner, and test code that ends the process after
/// printing a footer is indistinguishable from the runner's own. A pinned
/// unittest adapter that reports its own results would remove both.
fn package_script_unittest(
    package: &PackageBinding,
    command: &str,
    stderr: &str,
) -> Option<AssertionCounts> {
    static RAN: OnceLock<regex::Regex> = OnceLock::new();
    static RESULT: OnceLock<regex::Regex> = OnceLock::new();
    if !bare_package_test(command) || !package.runs_unittest || !package.unchanged() {
        return None;
    }
    let lines: Vec<_> = stderr
        .lines()
        .map(str::trim_end)
        .filter(|s| !s.is_empty())
        .collect();
    let [before @ .., rule, summary, verdict] = lines.as_slice() else {
        return None;
    };
    if *rule != "-".repeat(70) {
        return None;
    }
    let skipped: u32 = if *verdict == "OK" {
        0
    } else {
        verdict
            .strip_prefix("OK (skipped=")?
            .strip_suffix(')')?
            .parse()
            .ok()?
    };
    let pattern = RAN.get_or_init(|| {
        regex::Regex::new(r"^Ran (\d+) tests? in \d+\.\d{3}s$")
            .expect("static unittest summary pattern")
    });
    let total: u32 = pattern.captures(summary)?[1].parse().ok()?;
    let passed = total.checked_sub(skipped)?;

    let (recorded, recorded_skips) = match before.last() {
        // One character per test, written as each finished.
        Some(progress)
            if !progress.is_empty() && progress.bytes().all(|b| matches!(b, b'.' | b's')) =>
        {
            let skips = progress.bytes().filter(|b| *b == b's').count();
            (progress.len(), skips)
        }
        // Verbose: one `name ... outcome` line per test.
        _ => {
            let result = RESULT.get_or_init(|| {
                regex::Regex::new(r"^.+ \.\.\. (ok|skipped\b.*)$")
                    .expect("static unittest result pattern")
            });
            let results: Vec<_> = before
                .iter()
                .filter_map(|line| result.captures(line))
                .collect();
            let skips = results
                .iter()
                .filter(|captures| captures[1].starts_with("skipped"))
                .count();
            (results.len(), skips)
        }
    };
    if recorded != usize::try_from(total).ok()?
        || recorded_skips != usize::try_from(skipped).ok()?
    {
        return None;
    }
    Some(AssertionCounts {
        total,
        passed,
        failed: 0,
        skipped,
    })
}

const MAX_PACKAGE_JSON: u64 = 1024 * 1024;

fn read_manifest(cwd: &std::path::Path) -> Option<Vec<u8>> {
    davinci_sys::fs::read_path_bounded(&cwd.join("package.json"), MAX_PACKAGE_JSON).ok()
}

/// `<npm|pnpm|yarn> test` and nothing else on the line.
fn bare_package_test(command: &str) -> bool {
    super::literal_words(command, true)
        .or_else(|| super::literal_words(command, false))
        .is_some_and(|words| match words.as_slice() {
            [name, action] => matches!(name.as_str(), "npm" | "pnpm" | "yarn") && action == "test",
            _ => false,
        })
}

/// The `test` script in `manifest` is the standard unittest runner and nothing
/// else runs around it or reconfigures how scripts run.
fn script_is_unittest(cwd: &std::path::Path, manifest: &[u8]) -> bool {
    // These change which shell runs scripts, or which package a command means.
    const CONFIGURATION: [&str; 4] = [".npmrc", ".yarnrc", ".yarnrc.yml", ".pnpmrc"];
    if CONFIGURATION
        .iter()
        .any(|name| cwd.join(name).symlink_metadata().is_ok())
    {
        return false;
    }
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(manifest) else {
        return false;
    };
    if manifest.get("workspaces").is_some() {
        return false;
    }
    let scripts = &manifest["scripts"];
    if scripts.get("pretest").is_some() || scripts.get("posttest").is_some() {
        return false;
    }
    let Some(script) = scripts["test"].as_str() else {
        return false;
    };
    super::literal_words(script, true)
        .and_then(|words| runner_words(&words))
        .is_some_and(|runner| runner == "unittest")
}

fn pytest_summary(line: &str) -> Option<AssertionCounts> {
    static DURATION: OnceLock<regex::Regex> = OnceLock::new();
    if line.contains('\x1b') {
        return None;
    }
    let line = if line.starts_with('=') {
        line.strip_suffix('=')?.trim_matches('=').trim()
    } else {
        line
    };
    let (items, elapsed) = line.rsplit_once(" in ")?;
    let duration = DURATION.get_or_init(|| {
        regex::Regex::new(r"^\d+(?:\.\d+)?s(?: \(\d+:\d{2}:\d{2}\))?$")
            .expect("static pytest duration pattern")
    });
    if !duration.is_match(elapsed) {
        return None;
    }
    let mut counts = AssertionCounts::default();
    let mut seen = std::collections::BTreeSet::new();
    for item in items.split(", ") {
        if item == "no tests ran" {
            if !seen.insert("none") {
                return None;
            }
            continue;
        }
        let (number, word) = item.split_once(' ')?;
        let number = number.parse::<u32>().ok()?;
        let key = match word {
            "passed" | "failed" | "skipped" | "xfailed" | "xpassed" | "deselected" => word,
            "error" | "errors" => "error",
            "warning" | "warnings" => "warning",
            "rerun" | "reruns" => "rerun",
            _ => return None,
        };
        if !seen.insert(key) {
            return None;
        }
        if key == "deselected" && number > 0 {
            // Deselected tests never ran, so the summary says nothing about
            // the paths the command named.
            return None;
        }
        let field = match key {
            "passed" => &mut counts.passed,
            "failed" | "error" | "xpassed" => &mut counts.failed,
            "skipped" | "xfailed" => &mut counts.skipped,
            _ => continue,
        };
        *field = field.checked_add(number)?;
    }
    if seen.contains("none")
        && seen
            .iter()
            .any(|key| !matches!(*key, "none" | "warning" | "deselected"))
    {
        return None;
    }
    counts.total = counts
        .passed
        .checked_add(counts.failed)?
        .checked_add(counts.skipped)?;
    Some(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_prefixed_pytest_requires_discovery() {
        for command in [
            "python -m pytest -q",
            "PYTHONDONTWRITEBYTECODE=1 python -m pytest -q",
            "A=one B=two rtk proxy python -m pytest tests/test_changed.py -q",
        ] {
            let receipt = crate::runtime::evidence_store::ExecutionReceipt {
                argv: vec![command.into()],
                started: true,
                exit_code: Some(0),
                ..Default::default()
            };
            assert!(receipt.requires_test_discovery(), "{command}");
            assert!(!receipt.is_verified_check(), "{command}");
        }
    }

    #[test]
    fn environment_assignment_discovery_evaluation_preserves_runner_and_zero_test_gates() {
        let rust = b"test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.1s\n";
        let baseline = counts("cargo test --lib", rust, b"").unwrap();
        for command in [
            "A=one cargo test --lib",
            "A=one B=two rtk proxy cargo +1.88.0 test --lib",
            "rtk proxy A=one cargo test --lib",
        ] {
            assert_eq!(counts(command, rust, b"").unwrap(), baseline);
            let zero = b"test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n";
            assert!(!crate::verification::tests_passed(
                &counts(command, zero, b"").unwrap()
            ));
        }
        assert_eq!(
            counts(
                "A=one python -m unittest discover",
                b"",
                b"Ran 2 tests in 0.01s\n\nOK\n"
            )
            .unwrap()
            .passed,
            2
        );
        for command in [
            "A=one",
            "9INVALID=one cargo test",
            "A=one echo cargo test",
            "A=one cargo test || echo passed",
            "A=one pytest -q",
        ] {
            assert!(counts(command, rust, b"").is_none(), "{command}");
        }
    }

    #[test]
    fn pytest_final_summary_is_parsed_quiet_and_padded() {
        // Captured from bench-pubcmp-20261016 (t4-rename-r0, t8-calc-r0, t1-intervals-r2).
        let quiet = "......                                                                   [100%]\r\n6 passed in 0.01s\r\n";
        let padded = "collected 6 items\n\n====== 6 passed in 0.03s ======\n";
        let timed = "====== 6 passed in 1.20s (0:00:01) ======\n";
        for command in [
            "pytest -q",
            "python -m pytest -q",
            "PYTHONDONTWRITEBYTECODE=1 pytest -q",
        ] {
            for output in [quiet, padded, timed] {
                let result = counts(command, output.as_bytes(), b"").expect(command);
                assert_eq!((result.total, result.passed, result.failed), (6, 6, 0));
                assert!(crate::verification::tests_passed(&result));
            }
        }
        let mixed = counts(
            "pytest -q",
            b"3 passed, 1 skipped, 2 xfailed, 1 warning in 0.5s\n",
            b"",
        )
        .unwrap();
        assert_eq!(
            (mixed.total, mixed.passed, mixed.skipped, mixed.failed),
            (6, 3, 3, 0)
        );
        assert!(crate::verification::tests_passed(&mixed));
        // Deselected tests never ran: no evidence, not a pass.
        for output in [
            "3 passed, 1 skipped, 2 xfailed, 1 warning, 4 deselected in 0.5s\n",
            "1 passed, 2 deselected in 0.01s\n",
        ] {
            assert!(
                counts("pytest -q", output.as_bytes(), b"").is_none(),
                "{output}"
            );
        }
    }

    #[test]
    fn pytest_failures_errors_and_empty_runs_never_pass() {
        // The error run is the captured t4-rename-r0 collection failure.
        let collection = "=== ERRORS ===\r\nE   ModuleNotFoundError: No module named 'shop'\r\n!!!!!!!! Interrupted: 1 error during collection !!!!!!!!\r\n1 error in 0.07s\r\n";
        for (output, failed) in [
            ("1 failed, 5 passed in 0.03s\n", 1),
            ("=== 1 failed, 4 passed, 1 error in 0.03s ===\n", 2),
            ("1 xpassed, 5 passed in 0.03s\n", 1),
            (collection, 1),
        ] {
            let result = counts("pytest -q", output.as_bytes(), b"").unwrap();
            assert_eq!(result.failed, failed, "{output}");
            assert!(!crate::verification::tests_passed(&result), "{output}");
        }
        for output in [
            "no tests ran in 0.01s\n",
            "=== no tests ran in 0.01s ===\n",
            "no tests ran, 1 warning in 0.01s\n",
        ] {
            let result = counts("pytest -q", output.as_bytes(), b"").unwrap();
            assert_eq!(result.total, 0, "{output}");
            assert!(!crate::verification::tests_passed(&result), "{output}");
        }
        assert!(counts("pytest -q", b"3 deselected in 0.01s\n", b"").is_none());
    }

    #[test]
    fn pytest_rejects_summaries_that_are_not_the_final_clean_line() {
        for output in [
            "6 passed in 0.03s\nprinting more output\n",
            "6 passed in 0.03s\n6 passed in 0.03s extra\n",
            "6 passed in 0.03s\n\n",
            "6 passed, 1 flaky in 0.03s\n",
            "6 passed, 6 passed in 0.03s\n",
            "6 passed, 1 error, 1 errors in 0.03s\n",
            "6 passed in soon\n",
            "6 passed\n",
            "in 0.03s\n",
            "no tests ran, 6 passed in 0.03s\n",
            "4294967295 passed, 1 failed in 0.03s\n",
            "99999999999 passed in 0.03s\n",
            "\x1b[32m6 passed\x1b[0m in 0.03s\n",
            "=== 6 passed in 0.03s\n",
        ] {
            let accepted = counts("pytest -q", output.as_bytes(), b"");
            if output == "6 passed in 0.03s\n\n" {
                assert!(accepted.is_some(), "trailing blank lines are not output");
            } else {
                assert!(accepted.is_none(), "{output:?}");
            }
        }
        assert!(counts("pytest -q", b"", b"6 passed in 0.03s\n").is_none());
        for command in ["./tools/pytest -q", "npm test", "go test ./..."] {
            assert!(
                counts(command, b"6 passed in 0.03s\n", b"").is_none(),
                "{command}"
            );
        }
    }

    #[test]
    fn node_tap_requires_complete_consistent_nonempty_discovery() {
        let command = "node --test --test-reporter=tap tests/app.test.cjs";
        let summary = b"TAP version 13\n1..2\n# tests 2\n# suites 0\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 1\n# todo 0\n# duration_ms 12.5\n";
        let result = counts(command, summary, b"").expect("actual Node TAP summary");
        assert_eq!((result.total, result.passed, result.skipped), (2, 1, 1));
        assert!(counts(command, b"all tests passed", b"").is_none());
        assert!(counts("node --test tests/app.test.cjs", summary, b"").is_none());
        assert!(counts("echo node --test --test-reporter=tap", summary, b"").is_none());
        for invalid in [
            String::from_utf8_lossy(summary).replace("# pass 1", "# pass 3"),
            String::from_utf8_lossy(summary).replace("# todo 0\n", ""),
            String::from_utf8_lossy(summary).replace("# fail 0", "# fail 0\n# fail 0"),
        ] {
            assert!(counts(command, invalid.as_bytes(), b"").is_none());
        }
        let zero = b"TAP version 13\n1..0\n# tests 0\n# suites 0\n# pass 0\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n# duration_ms 1\n";
        assert!(!crate::verification::tests_passed(
            &counts(command, zero, b"").unwrap()
        ));
    }
    #[test]
    fn standard_unittest_summaries_preserve_actual_pass_and_skip_counts() {
        let command = "python -m unittest discover";
        for (summary, total, passed, skipped) in [
            ("Ran 8 tests in 0.01s\n\nOK\n", 8, 8, 0),
            ("Ran 8 tests in 0.01s\n\nOK (skipped=4)\n", 8, 4, 4),
            ("Ran 4 tests in 0.01s\n\nOK (skipped=4)\n", 4, 0, 4),
            ("Ran 0 tests in 0.01s\n\nOK\n", 0, 0, 0),
        ] {
            let result = counts(command, b"", summary.as_bytes()).unwrap();
            assert_eq!(
                (result.total, result.passed, result.skipped),
                (total, passed, skipped)
            );
            assert_eq!(crate::verification::tests_passed(&result), passed > 0);
        }
        for summary in [
            "Ran 8 tests in 0.01s\n\nFAILED (failures=1)\n",
            "Ran 3 tests in 0.01s\n\nOK (skipped=4)\n",
            "Ran 8 tests in 0.01s\n\nOK (skipped=invalid)\n",
            "Ran 8 tests in 0.01s\n\nOK (skipped=4294967296)\n",
            "Ran 8 tests in 0.01s\n\nOK (unexpected successes=1)\n",
        ] {
            assert!(counts(command, b"", summary.as_bytes()).is_none());
        }
    }

    #[test]
    fn harness_zero_tests_and_echoed_pass_are_not_test_success() {
        let zero = b"test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.00s\n";
        assert_eq!(
            counts("cargo test missing_filter", zero, b"")
                .unwrap()
                .total,
            0
        );
        assert!(counts("echo cargo test", zero, b"").is_none());
        assert!(counts("cargo test; echo ok", zero, b"").is_none());
        assert!(counts("cargo test", b"all tests passed", b"").is_none());
        let passed = b"test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.1s\n";
        let result = counts("cargo test --workspace", passed, b"").unwrap();
        assert_eq!((result.total, result.passed, result.skipped), (3, 2, 1));
        assert_eq!(
            counts(
                "python -m unittest discover",
                b"",
                b"Ran 0 tests in 0.00s\n\nOK\n"
            )
            .unwrap()
            .total,
            0
        );
        assert!(counts("pytest", b"unknown format", b"").is_none());
    }

    fn bind(dir: &tempfile::TempDir) -> Option<PackageBinding> {
        PackageBinding::bind(dir.path())
    }

    /// A tempdir holding a `package.json` whose `test` script is `script`.
    fn package(script: &str, extra: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            format!(r#"{{"name":"app","scripts":{{"test":{script:?}{extra}}}}}"#),
        )
        .unwrap();
        dir
    }

    fn unittest_stderr(progress: &str, summary: &str) -> Vec<u8> {
        format!("{progress}\n{}\n{summary}\n", "-".repeat(70)).into_bytes()
    }

    #[test]
    fn audit_wp02_b032_npm_wrapper_accepts_only_real_unittest_evidence() {
        let npm = "npm test";
        let stdout = b"> app@1.0.0 test\n> python -m unittest\n";
        let dir = package("python -m unittest", "");
        let at = |command: &str, err: &[u8]| counts_in(bind(&dir).as_ref(), command, stdout, err);

        let real = unittest_stderr("...", "Ran 3 tests in 0.002s\n\nOK");
        let result = at(npm, &real).expect("real unittest evidence");
        assert_eq!((result.total, result.passed, result.failed), (3, 3, 0));
        assert!(crate::verification::tests_passed(&result));
        for wrapper in ["pnpm test", "yarn test"] {
            assert!(at(wrapper, &real).is_some(), "{wrapper}");
        }
        let skipped = unittest_stderr("..s.", "Ran 4 tests in 0.010s\n\nOK (skipped=1)");
        let result = at(npm, &skipped).unwrap();
        assert_eq!((result.total, result.passed, result.skipped), (4, 3, 1));
        let verbose = unittest_stderr(
            "test_a (t.T.test_a) ... ok\ntest_b (t.T.test_b) ... skipped 'later'",
            "Ran 2 tests in 0.001s\n\nOK (skipped=1)",
        );
        let result = at(npm, &verbose).unwrap();
        assert_eq!((result.total, result.passed, result.skipped), (2, 1, 1));

        // Zero tests run: counted, but never a passing check.
        let none = unittest_stderr("", "Ran 0 tests in 0.000s\n\nOK");
        assert!(!crate::verification::tests_passed(&at(npm, &none).unwrap()));
        // A failing run carries no OK verdict.
        let failed = unittest_stderr("F..", "Ran 3 tests in 0.002s\n\nFAILED (failures=1)");
        assert!(at(npm, &failed).is_none());
    }

    /// The review's exact bypass: a script that exits zero and prints only the
    /// footer to stderr ran no tests and must not count as three passing ones.
    #[test]
    fn audit_wp02_b032_a_printed_footer_is_not_discovery() {
        let footer = unittest_stderr_footer_only();
        let stdout = b"";
        // The exact three lines, with every other safeguard in place: the
        // package really names the unittest runner.
        let unittest_dir = package("python -m unittest", "");
        assert!(counts_in(bind(&unittest_dir).as_ref(), "npm test", stdout, &footer).is_none());
        // And with no package at all, or no directory.
        let empty = tempfile::tempdir().unwrap();
        assert!(counts_in(bind(&empty).as_ref(), "npm test", stdout, &footer).is_none());
        assert!(counts("npm test", stdout, &footer).is_none());

        // Footer plus a plausible progress line, but the script is `echo`:
        // identity fails.
        let with_progress = unittest_stderr("...", "Ran 3 tests in 0.002s\n\nOK");
        for script in [
            "echo spoof",
            "python -m unittest && echo ok",
            "python -m unittest || true",
            "node fake.js",
            "python -m pytest",
        ] {
            let dir = package(script, "");
            assert!(
                counts_in(bind(&dir).as_ref(), "npm test", stdout, &with_progress).is_none(),
                "{script}"
            );
        }
        // Hooks that run around the script write to the same streams.
        for hook in [r#","posttest":"echo done""#, r#","pretest":"echo start""#] {
            let dir = package("python -m unittest", hook);
            assert!(counts_in(bind(&dir).as_ref(), "npm test", stdout, &with_progress).is_none());
        }
        // The progress records must reconcile with the summary.
        for (progress, summary) in [
            ("..", "Ran 3 tests in 0.002s\n\nOK"),
            ("....", "Ran 3 tests in 0.002s\n\nOK"),
            ("..s", "Ran 3 tests in 0.002s\n\nOK"),
            ("...", "Ran 3 tests in 0.002s\n\nOK (skipped=1)"),
            ("", "Ran 3 tests in 0.002s\n\nOK"),
        ] {
            assert!(
                counts_in(
                    bind(&unittest_dir).as_ref(),
                    "npm test",
                    stdout,
                    &unittest_stderr(progress, summary)
                )
                .is_none(),
                "{progress:?} {summary:?}"
            );
        }
        assert!(counts_in(
            bind(&unittest_dir).as_ref(),
            "npm test",
            stdout,
            &unittest_stderr("test_a (t.T.test_a) ... ok", "Ran 3 tests in 0.002s\n\nOK")
        )
        .is_none());
        // Printed text without the runner's report: stdout, no rule, wrong timing.
        let spoofs: [&[u8]; 4] = [
            b"",
            b"----------\nRan 3 tests in 0.002s\n\nOK\n",
            b"...\n----------------------------------------------------------------------\nRan 3 tests in 0.1s\n\nOK\n",
            b"...\n----------------------------------------------------------------------\nRan 3 tests in 0.002s\n\nOK\nextra line\n",
        ];
        for err in spoofs {
            assert!(
                counts_in(bind(&unittest_dir).as_ref(), "npm test", stdout, err).is_none(),
                "{err:?}"
            );
        }
        assert!(counts_in(
            bind(&unittest_dir).as_ref(),
            "npm test",
            b"Ran 3 tests in 0.002s\n\nOK\n",
            b""
        )
        .is_none());
        // The wrapper does not weaken other runners or the direct command.
        assert!(counts_in(
            bind(&unittest_dir).as_ref(),
            "go test ./...",
            stdout,
            &with_progress
        )
        .is_none());
        assert!(counts("python -m unittest", b"", &with_progress).is_some());
    }

    fn unittest_stderr_footer_only() -> Vec<u8> {
        format!("{}\nRan 3 tests in 0.002s\n\nOK\n", "-".repeat(70)).into_bytes()
    }

    /// A wrapper that fails cannot be verified execution, however good its
    /// discovery output looks: the receipt gate is the exit status.
    #[test]
    fn audit_wp02_b032_a_nonzero_wrapper_exit_is_never_a_verified_check() {
        let dir = package("python -m unittest", "");
        let stderr = unittest_stderr("...", "Ran 3 tests in 0.002s\n\nOK");
        let counts = counts_in(bind(&dir).as_ref(), "npm test", b"", &stderr).unwrap();
        let receipt =
            |exit: Option<i32>, started: bool| crate::runtime::evidence_store::ExecutionReceipt {
                argv: vec!["npm test".into()],
                started,
                exit_code: exit,
                assertion_counts: Some(counts.clone()),
                ..Default::default()
            };
        assert!(receipt(Some(0), true).is_verified_check());
        for broken in [
            receipt(Some(1), true),
            receipt(None, true),
            receipt(Some(0), false),
        ] {
            assert!(!broken.is_verified_check());
        }
    }

    /// The review's second bypass: the script is not unittest when it starts, and
    /// rewrites `scripts.test` while it runs. Identity comes from the manifest
    /// bound before the process, so the rewrite buys nothing; a manifest that
    /// really was unittest and then changes is refused as well.
    #[test]
    fn audit_wp02_b032_a_manifest_rewritten_during_the_run_is_not_identity() {
        let stderr = unittest_stderr(
            "...",
            "Ran 3 tests in 0.002s

OK",
        );
        let manifest =
            |script: &str| format!(r#"{{"name":"app","scripts":{{"test":{script:?}}}}}"#);

        let dir = package("node fake.js", "");
        let bound = bind(&dir);
        std::fs::write(
            dir.path().join("package.json"),
            manifest("python -m unittest"),
        )
        .unwrap();
        assert!(counts_in(bound.as_ref(), "npm test", b"", &stderr).is_none());
        // Bound afterwards, the same bytes would have passed: that is the hole.
        assert!(counts_in(bind(&dir).as_ref(), "npm test", b"", &stderr).is_some());

        let dir = package("python -m unittest", "");
        let bound = bind(&dir);
        assert!(counts_in(bound.as_ref(), "npm test", b"", &stderr).is_some());
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"python -m unittest","posttest":"echo x"}}"#,
        )
        .unwrap();
        assert!(counts_in(bound.as_ref(), "npm test", b"", &stderr).is_none());
        std::fs::remove_file(dir.path().join("package.json")).unwrap();
        assert!(counts_in(bound.as_ref(), "npm test", b"", &stderr).is_none());
    }

    /// Package redirection the host does not model: the manifest next to the
    /// command is unittest, but the command or its configuration points the
    /// package manager somewhere else.
    #[test]
    fn audit_wp02_b032_package_redirection_is_not_identity() {
        let stderr = unittest_stderr(
            "...",
            "Ran 3 tests in 0.002s

OK",
        );
        let dir = package("python -m unittest", "");
        let bound = bind(&dir);
        assert!(counts_in(bound.as_ref(), "npm test", b"", &stderr).is_some());
        // Still a recognised wrapper (so discovery is required), never evidence.
        for command in [
            "npm test --prefix fake",
            "npm test -C fake",
            "npm test --workspace fake",
            "npm test -- --other",
            "yarn test --cwd fake",
            "pnpm test --dir fake",
            "npm_config_prefix=fake npm test",
            "rtk proxy npm test",
            "npm test && echo done",
        ] {
            assert!(
                crate::verification::discovery::runner(command).is_some() || command.contains("&&"),
                "{command}"
            );
            assert!(
                counts_in(bound.as_ref(), command, b"", &stderr).is_none(),
                "{command}"
            );
        }
        // Configuration beside the manifest can change the shell scripts run in.
        for name in [".npmrc", ".yarnrc", ".yarnrc.yml", ".pnpmrc"] {
            let configured = package("python -m unittest", "");
            std::fs::write(configured.path().join(name), "script-shell=fake").unwrap();
            assert!(
                counts_in(bind(&configured).as_ref(), "npm test", b"", &stderr).is_none(),
                "{name}"
            );
        }
        let workspaces = package("python -m unittest", "");
        std::fs::write(
            workspaces.path().join("package.json"),
            r#"{"workspaces":["fake"],"scripts":{"test":"python -m unittest"}}"#,
        )
        .unwrap();
        assert!(counts_in(bind(&workspaces).as_ref(), "npm test", b"", &stderr).is_none());
    }

    /// Positive control from a real `npm test` that runs a real unittest suite.
    /// Skipped, and said so, where npm or python is not installed.
    #[test]
    fn audit_wp02_b032_a_real_npm_run_of_unittest_is_verified() {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let python = if cfg!(windows) { "python" } else { "python3" };
        let present = |program: &str, flag: &str| {
            std::process::Command::new(program)
                .arg(flag)
                .output()
                .is_ok_and(|out| out.status.success())
        };
        if !present(npm, "--version") || !present(python, "--version") {
            eprintln!("npm or python missing; real npm-to-unittest fixture not executed");
            return;
        }
        let dir = package(&format!("{python} -m unittest"), "");
        std::fs::write(
            dir.path().join("test_app.py"),
            "import unittest\nclass T(unittest.TestCase):\n    def test_a(self):\n        self.assertTrue(True)\n    @unittest.skip('later')\n    def test_b(self):\n        pass\n    def test_c(self):\n        self.assertEqual(1, 1)\n",
        )
        .unwrap();
        let output = std::process::Command::new(npm)
            .arg("test")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = counts_in(
            bind(&dir).as_ref(),
            "npm test",
            &output.stdout,
            &output.stderr,
        )
        .expect("a real unittest run through npm");
        assert_eq!(
            (result.total, result.passed, result.skipped, result.failed),
            (3, 2, 1, 0)
        );
        assert!(crate::verification::tests_passed(&result));
        // The same real output, replayed where the package names another script.
        let other = package("echo hi", "");
        assert!(counts_in(
            bind(&other).as_ref(),
            "npm test",
            &output.stdout,
            &output.stderr
        )
        .is_none());
    }
}

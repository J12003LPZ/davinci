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
        "package-script" => return package_script_unittest(stderr),
        "pytest" => {
            // Only pytest's final terminal summary; earlier lines can be test output.
            let end = stdout.lines().map(str::trim).rfind(|s| !s.is_empty())?;
            return pytest_summary(end);
        }
        _ => return None,
    }
    Some(counts)
}

/// A package-manager `test` script that ran Python's standard unittest runner.
/// The runner writes its report to stderr, ending in a rule of 70 dashes, a
/// `Ran N tests in S.SSSs` line and the `OK` verdict. All of it must close the
/// stream, so a summary printed to stdout, or one without the rule and the
/// runner's timing format, proves nothing. Any other runner stays unknown.
fn package_script_unittest(stderr: &str) -> Option<AssertionCounts> {
    static RAN: OnceLock<regex::Regex> = OnceLock::new();
    let lines: Vec<_> = stderr
        .lines()
        .map(str::trim_end)
        .filter(|s| !s.is_empty())
        .collect();
    let [.., rule, summary, verdict] = lines.as_slice() else {
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
    Some(AssertionCounts {
        total,
        passed: total.checked_sub(skipped)?,
        failed: 0,
        skipped,
    })
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

    fn unittest_stderr(summary: &str) -> Vec<u8> {
        format!("...\n{}\n{summary}\n", "-".repeat(70)).into_bytes()
    }

    #[test]
    fn audit_wp02_b032_npm_wrapper_accepts_only_real_unittest_evidence() {
        let npm = "npm test";
        let stdout = b"> app@1.0.0 test\n> python -m unittest\n";
        let real = unittest_stderr("Ran 3 tests in 0.002s\n\nOK");
        let counts = counts(npm, stdout, &real).expect("real unittest evidence");
        assert_eq!((counts.total, counts.passed, counts.failed), (3, 3, 0));
        assert!(crate::verification::tests_passed(&counts));
        for wrapper in ["pnpm test", "yarn test --silent"] {
            assert!(super::counts(wrapper, stdout, &real).is_some(), "{wrapper}");
        }
        let skipped = unittest_stderr("Ran 4 tests in 0.010s\n\nOK (skipped=1)");
        let counts = super::counts(npm, stdout, &skipped).unwrap();
        assert_eq!((counts.total, counts.passed, counts.skipped), (4, 3, 1));

        // Zero tests run: counted, but never a passing check.
        let none = unittest_stderr("Ran 0 tests in 0.000s\n\nOK");
        let zero = super::counts(npm, stdout, &none).unwrap();
        assert!(!crate::verification::tests_passed(&zero));
        // A failing run carries no OK verdict.
        let failed = unittest_stderr("Ran 3 tests in 0.002s\n\nFAILED (failures=1)");
        assert!(super::counts(npm, stdout, &failed).is_none());

        // Printed text without the runner's report: stdout, no rule, wrong timing.
        let spoofs: [(&[u8], &[u8]); 5] = [
            (b"Ran 3 tests in 0.002s\n\nOK\n", b""),
            (b"", b"Ran 3 tests in 0.002s\n\nOK\n"),
            (b"", b"----------\nRan 3 tests in 0.002s\n\nOK\n"),
            (b"", b"----------------------------------------------------------------------\nRan 3 tests in 0.1s\n\nOK\n"),
            (b"", b"----------------------------------------------------------------------\nRan 3 tests in 0.002s\n\nOK\nextra line\n"),
        ];
        for (out, err) in spoofs {
            assert!(super::counts(npm, out, err).is_none(), "{err:?}");
        }
        // The wrapper does not weaken other runners or the direct command.
        assert!(super::counts("go test ./...", stdout, &real).is_none());
        assert!(super::counts("python -m unittest", b"", &real).is_some());
    }
}

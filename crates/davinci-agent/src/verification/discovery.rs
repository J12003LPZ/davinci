//! Discovery from actual captured runner output, never decorated tool text.
use crate::runtime::evidence::AssertionCounts;
use std::sync::OnceLock;

pub(crate) fn runner(command: &str) -> Option<&'static str> {
    let words =
        super::literal_words(command, true).or_else(|| super::literal_words(command, false))?;
    runner_words(&words)
}

pub(crate) fn runner_words(words: &[String]) -> Option<&'static str> {
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
        [name, action, ..] if name == "cargo" && action == "test" => Some("rust"),
        [name, module, runner, ..]
            if super::python_name(name) && module == "-m" && runner == "unittest" =>
        {
            Some("unittest")
        }
        [name, module, runner, ..]
            if super::python_name(name) && module == "-m" && runner == "pytest" =>
        {
            Some("unknown")
        }
        [name, ..] if matches!(name.as_str(), "pytest" | "pytest.exe") => Some("unknown"),
        [name, action, ..]
            if matches!(name.as_str(), "go" | "npm" | "pnpm" | "yarn") && action == "test" =>
        {
            Some("unknown")
        }
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
        _ => return None,
    }
    Some(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}

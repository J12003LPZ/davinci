use davinci_agent::verification::{classify, CheckKind};
use std::path::PathBuf;

#[test]
fn inline_import_has_exact_source_coverage() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "def f(x): return x * 2\n").unwrap();
    let paths = vec![PathBuf::from("changed.py"), PathBuf::from("other.py")];
    let result = classify(
        "bash",
        "python -c 'from changed import f; assert f(2) == 4'",
        root.path(),
        &paths,
    );
    assert_eq!(result.kind, CheckKind::TargetedScript);
    assert_eq!(result.covered, vec![PathBuf::from("changed.py")]);
    assert!(!result.complete);
}

#[test]
fn noop_masked_background_and_disabled_checks_are_unknown() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "x = 1\n").unwrap();
    for command in [
        "python -c 'import changed; assert True'",
        "python -O -c 'import changed; assert changed.x == 1'",
        "python -c 'import changed; assert changed.x == 1' || true",
        "python -c 'import changed; assert changed.x == 1' &",
        "python -c 'import changed; assert changed.x == 1'; true",
        "python -c 'import changed; assert changed.x == 1' | cat",
        "cargo check --help",
        "pytest --collect-only",
        "cargo test --no-run",
        "cargo test -- --list",
        "pytest --fixtures",
        "pytest --setup-plan",
    ] {
        let result = classify("bash", command, root.path(), &[PathBuf::from("changed.py")]);
        assert!(!result.complete, "{command}");
        assert!(result.covered.is_empty(), "{command}");
    }
}

#[test]
fn quoted_heredoc_and_compile_chain_preserve_literal_punctuation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "x = ';|&'\n").unwrap();
    let command = "python -m py_compile changed.py && python - <<'PY'\nimport changed\nassert changed.x == ';|&'\nPY";
    let result = classify("bash", command, root.path(), &[PathBuf::from("changed.py")]);
    assert!(result.complete, "{result:?}");
    assert!(
        !classify(
            "powershell",
            command,
            root.path(),
            &[PathBuf::from("changed.py")]
        )
        .complete
    );
}

#[test]
fn changed_dependency_and_test_file_are_covered_but_sibling_is_not() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "from dependency import f\n").unwrap();
    std::fs::write(
        root.path().join("dependency.py"),
        "def f(x): return x * 2\n",
    )
    .unwrap();
    let paths = [
        "changed.py",
        "dependency.py",
        "test_changed.py",
        "sibling.py",
    ]
    .map(PathBuf::from);
    let result = classify(
        "bash",
        "python -c 'from changed import f; assert f(2) == 4'",
        root.path(),
        &paths,
    );
    assert_eq!(result.covered, paths[..3]);
    assert!(!result.complete);
}

#[test]
fn suite_in_subdirectory_cannot_cover_sibling_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    let paths = ["sub/a.py", "other.py"].map(PathBuf::from);
    let result = classify("bash", "cd sub && pytest -q", root.path(), &paths);
    assert_eq!(result.covered, vec![paths[0].clone()]);
    assert!(!result.complete);
}

#[test]
#[ignore = "diagnostic timing; run alone without build or campaign contention"]
fn classification_overhead_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("measured.py"), "def twice(x): return x * 2\n").unwrap();
    let command = "python -c 'from measured import twice; assert twice(3) == 6'";
    let paths = [PathBuf::from("measured.py")];
    let start = std::time::Instant::now();
    assert!(classify("bash", command, root.path(), &paths).complete);
    let cold_us = start.elapsed().as_micros();
    let mut samples = Vec::new();
    for _ in 0..100 {
        let start = std::time::Instant::now();
        assert!(classify("bash", command, root.path(), &paths).complete);
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    println!("classification cold_us={cold_us} cached_median_us={} cached_p95_us={}", samples[50], samples[95]);
}

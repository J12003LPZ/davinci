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
fn changed_dependency_is_covered_but_unexecuted_tests_and_siblings_are_not() {
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
    assert_eq!(result.covered, paths[..2]);
    assert!(!result.complete);
}

#[test]
fn source_in_docs_tests_and_fixtures_is_never_blanket_exempt() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("docs/project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("changed.py"), "x = 1").unwrap();
    let paths = [
        "changed.py",
        "other.py",
        "docs/example.py",
        "tests/helper.py",
        "fixtures/input.py",
        "README.md",
    ]
    .map(|path| workspace.join(path));
    let result = classify(
        "bash",
        "python -c 'import changed; assert changed.x == 1'",
        &workspace,
        &paths,
    );
    assert_eq!(result.covered, vec![paths[5].clone(), paths[0].clone()]);
    assert!(!result.complete);
}

#[test]
fn package_submodule_multiline_source_and_deleted_paths_have_exact_coverage() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pkg")).unwrap();
    std::fs::write(root.path().join("pkg/__init__.py"), "").unwrap();
    std::fs::write(root.path().join("pkg/lru.py"), "def f(x): return x * 2").unwrap();
    let paths = ["pkg/lru.py", "other/lru.py"].map(PathBuf::from);
    let result = classify(
        "bash",
        "python -c 'from pkg import lru\nif lru.f(2) != 4:\n raise AssertionError'",
        root.path(),
        &paths,
    );
    assert_eq!(result.covered, vec![paths[0].clone()]);
    assert!(!result.complete);
    let deleted = [PathBuf::from("deleted.py")];
    assert!(
        classify(
            "bash",
            "python -c 'from pathlib import Path; assert not Path(\"deleted.py\").exists()'",
            root.path(),
            &deleted
        )
        .complete
    );
    assert!(classify("bash", "pytest -q", root.path(), &deleted).complete);
}

#[test]
fn powershell_uses_its_own_literal_quoting_and_rejects_bash_control() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "x = 'value'").unwrap();
    let paths = [PathBuf::from("changed.py")];
    let result = classify(
        "powershell",
        "python -c 'import changed; assert changed.x == ''value'''",
        root.path(),
        &paths,
    );
    assert!(result.complete, "{result:?}");
    for command in [
        "pytest -q && pytest -q",
        "cd child && pytest",
        "python - <<'PY'\nassert True\nPY",
    ] {
        assert_eq!(
            classify("powershell", command, root.path(), &paths).kind,
            CheckKind::Unknown,
            "{command}"
        );
    }
}

#[test]
fn filtered_suites_do_not_claim_unknown_workspace_coverage() {
    let root = tempfile::tempdir().unwrap();
    for command in [
        "pytest tests/test_a.py",
        "pytest -k selected",
        "cargo test -p one",
        "cargo test one_test",
    ] {
        assert!(
            !classify("bash", command, root.path(), &[]).full_workspace,
            "{command}"
        );
    }
    assert!(classify(
        "bash",
        "pytest -k selected",
        root.path(),
        &[PathBuf::from("changed.py")]
    )
    .covered
    .is_empty());
    assert!(classify(
        "bash",
        "cargo test one_test",
        root.path(),
        &[PathBuf::from("src/lib.rs")]
    )
    .covered
    .is_empty());
    assert!(classify("bash", "pytest -q", root.path(), &[]).full_workspace);
}

#[cfg(unix)]
#[test]
fn symlinked_working_directory_covers_existing_and_deleted_workspace_paths() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("outer/real");
    let alias = root.path().join("alias");
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    std::fs::write(real.join("existing.py"), "x = 1").unwrap();
    let paths = [real.join("existing.py"), real.join("deleted.py")];
    let result = classify("bash", "pytest -q", &alias, &paths);
    assert!(result.complete, "{result:?}");
    let paths = [real.join("deleted.py"), root.path().join("sibling.py")];
    let result = classify("bash", "pytest -q", &alias.join(".."), &paths);
    assert_eq!(result.covered, vec![paths[0].clone()]);
}

#[cfg(windows)]
#[test]
fn windows_case_alias_and_bash_heredoc_match_the_actual_shell() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.py"), "x = 1").unwrap();
    let alias = PathBuf::from(root.path().to_string_lossy().to_uppercase());
    let paths = [root.path().join("changed.py")];
    assert!(classify("powershell", "pytest -q", &alias, &paths).complete);
    let command = "python - <<'PY'\nimport changed\nassert changed.x == 1\nPY";
    assert!(classify("bash", command, &alias, &paths).complete);
    assert_eq!(
        classify("exec_command", command, &alias, &paths).kind,
        CheckKind::Unknown
    );
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
    std::fs::write(
        root.path().join("measured.py"),
        "def twice(x): return x * 2\n",
    )
    .unwrap();
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
    println!(
        "classification cold_us={cold_us} cached_median_us={} cached_p95_us={}",
        samples[50], samples[95]
    );
}

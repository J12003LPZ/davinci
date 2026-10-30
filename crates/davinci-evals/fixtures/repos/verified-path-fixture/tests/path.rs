use verified_path::is_safe_relative;

#[test]
fn accepts_a_relative_path() {
    assert!(is_safe_relative("src/lib.rs"));
}

#[test]
fn rejects_parent_components() {
    assert!(!is_safe_relative("../secrets.txt"));
    assert!(!is_safe_relative("src/../../secrets.txt"));
}

#[test]
fn rejects_absolute_paths() {
    assert!(!is_safe_relative("/etc/passwd"));
}
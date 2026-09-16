use davinci_coding_agent::native_extensions::{
    SecurityScanController, SecurityVerifyRequest,
};
use std::fs;

#[test]
fn security_scan_incremental_telemetry() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("one.rs"), "pub fn one() {}\n").unwrap();
    fs::write(tmp.path().join("two.rs"), "pub fn two() {}\n").unwrap();
    let files = vec!["one.rs".to_string(), "two.rs".to_string()];
    let mut controller = SecurityScanController::new(tmp.path().to_path_buf());

    controller
        .verify_changed_surface(SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &files,
            graph_run_id: "cold",
        })
        .unwrap();
    let cold = controller.current().unwrap().coverage;
    assert_eq!(
        (cold.files_scanned_cold, cold.files_reused, cold.files_rescanned),
        (2, 0, 0)
    );
    assert_eq!((cold.cache_read_errors, cold.cache_write_errors), (0, 0));

    controller
        .verify_changed_surface(SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &files,
            graph_run_id: "warm",
        })
        .unwrap();
    let warm = controller.current().unwrap().coverage;
    assert_eq!(
        (warm.files_scanned_cold, warm.files_reused, warm.files_rescanned),
        (0, 2, 0)
    );
    assert_eq!((warm.cache_read_errors, warm.cache_write_errors), (0, 0));

    fs::write(tmp.path().join("one.rs"), "pub fn one() { eval(input); }\n").unwrap();
    controller
        .verify_changed_surface(SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &files,
            graph_run_id: "changed",
        })
        .unwrap();
    let changed = controller.current().unwrap().coverage;
    assert_eq!(
        (
            changed.files_scanned_cold,
            changed.files_reused,
            changed.files_rescanned,
        ),
        (0, 1, 1)
    );
    assert_eq!(
        (changed.cache_read_errors, changed.cache_write_errors),
        (0, 0)
    );
}

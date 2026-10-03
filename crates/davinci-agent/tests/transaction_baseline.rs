//! Frozen before measurement. Run explicitly; it uses only temporary files.
use davinci_agent::{apply_patch, tools};
use serde_json::json;
use std::fs;
use std::time::Instant;

#[test]
#[ignore = "explicit pre-P4 transaction behavior measurement"]
fn transaction_before_measurement() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "before\n").unwrap();
    fs::write(root.path().join("b.txt"), "before\n").unwrap();
    let began = Instant::now();
    let written = tools::execute_tool(
        root.path(),
        "write",
        &json!({"path":"a.txt","content":"written\n"}),
    )
    .unwrap();
    assert!(!written.is_error);
    let edited = tools::execute_tool(
        root.path(),
        "edit",
        &json!({"path":"a.txt","oldText":"written","newText":"edited"}),
    )
    .unwrap();
    assert!(!edited.is_error);
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-edited\n+patched\n*** Update File: b.txt\n@@\n-before\n+patched\n*** End Patch";
    let patched = tools::execute_tool(root.path(), "apply_patch", &json!({"input":patch})).unwrap();
    assert!(!patched.is_error, "{}", patched.content);
    let mutation_ms = began.elapsed().as_secs_f64() * 1000.0;
    let journal_after_success = root.path().join(apply_patch::JOURNAL_FILE_NAME).exists();
    let journal = apply_patch::PatchJournal {
        timestamp: 1,
        entries: vec![apply_patch::JournalEntry {
            relative_path: "a.txt".into(),
            original_content: Some("before\n".into()),
        }],
    };
    fs::write(
        root.path().join(apply_patch::JOURNAL_FILE_NAME),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("a.txt"), "later user bytes\n").unwrap();
    let recovery = apply_patch::recover_incomplete_journal_if_any(root.path());
    let artifact = json!({
        "schema":1,"platform":std::env::consts::OS,"measurement":"P4 before: public write/edit/apply_patch and preimage-only journal recovery in temporary files",
        "operation_count":3,"affected_files":2,"mutation_ms":mutation_ms,
        "write_has_transaction_id":written.details.as_ref().and_then(|v| v.get("transaction_id")).is_some(),
        "edit_has_transaction_id":edited.details.as_ref().and_then(|v| v.get("transaction_id")).is_some(),
        "patch_has_transaction_id":patched.details.as_ref().and_then(|v| v.get("transaction_id")).is_some(),
        "journal_after_success":journal_after_success,
        "legacy_recovery_succeeded":recovery.is_ok(),
        "later_user_bytes_preserved":fs::read(root.path().join("a.txt")).unwrap()==b"later user bytes\n",
        "notes":["A constructed incomplete legacy journal reproduces the public recovery contract; no crash is simulated in this baseline.","Source byte equality is measured directly. Timing is a single debug-profile sample."]
    });
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
    if let Some(path) = std::env::var_os("DAVINCI_TRANSACTION_EVAL_ARTIFACT") {
        fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}

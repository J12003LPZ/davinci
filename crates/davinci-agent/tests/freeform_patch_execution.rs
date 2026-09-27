use std::fs;

use davinci_agent::apply_patch::{self, FileAction};
use davinci_agent::runtime::contracts::extract_tool_targets;
use davinci_agent::tools::{execute_tool_with, tool_specs, ToolContext};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn parser_accepts_supported_codex_forms_without_normalizing_content() {
    let patch = "*** Begin Patch\r\n\
*** Add File: empty.txt\r\n\
*** End of File\r\n\
*** Add File: literal.txt\r\n\
+*** Update File: this-is-content\r\n\
+*** End Patch is content\r\n\
*** Update File: unicode.txt\r\n\
@@ replace\r\n\
-old α\r\n\
+new \"β\" / slash\r\n\
*** End of File\r\n\
*** End Patch ***";

    let parsed = apply_patch::parse_codex_patch(patch).expect("valid Codex patch");
    assert_eq!(parsed.actions.len(), 3);
    assert!(matches!(
        &parsed.actions[0],
        FileAction::Add { path, content } if path == "empty.txt" && content.is_empty()
    ));
    assert!(matches!(
        &parsed.actions[1],
        FileAction::Add { path, content }
            if path == "literal.txt"
                && content.contains("*** Update File: this-is-content")
                && content.contains("*** End Patch is content")
    ));
    assert!(matches!(
        &parsed.actions[2],
        FileAction::Update { path, hunks }
            if path == "unicode.txt"
                && hunks.len() == 1
                && hunks[0].lines.len() == 2
    ));

    let no_final_lf = "*** Begin Patch\n*** Add File: no-final-lf.txt\n+data\n*** End Patch";
    assert!(apply_patch::parse_codex_patch(no_final_lf).is_ok());
}

#[test]
fn parser_rejects_malformed_or_unsupported_controls_before_execution() {
    let malformed = "*** Begin Patch\n*** Update File: target.txt\n-old\n+new\n*** End Patch";
    let error = apply_patch::parse_codex_patch(malformed).unwrap_err();
    assert!(error.contains("@@"), "{error}");

    let moved = "*** Begin Patch\n*** Move to: renamed.txt\n*** End Patch";
    let error = apply_patch::parse_codex_patch(moved).unwrap_err();
    assert!(error.contains("*** Move to:"), "{error}");
    assert!(error.to_ascii_lowercase().contains("delete plus an add"), "{error}");

    let root = tempdir().unwrap();
    let result = execute_tool_with(
        root.path(),
        "apply_patch",
        &json!({"input": malformed}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(result.is_error);
    assert!(!root.path().join("target.txt").exists());
}

#[test]
fn decoded_freeform_and_json_apply_patch_share_targets_and_transaction_effects() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** Add File: b.txt\n+β\n*** End Patch";
    let json_input = json!({"input": patch});
    let targets = extract_tool_targets("apply_patch", &json_input);
    assert_eq!(targets, vec!["a.txt", "b.txt"]);

    let json_root = tempdir().unwrap();
    fs::write(json_root.path().join("a.txt"), "old\n").unwrap();
    let json_result = execute_tool_with(
        json_root.path(),
        "apply_patch",
        &json_input,
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!json_result.is_error, "{}", json_result.content);

    let freeform_root = tempdir().unwrap();
    fs::write(freeform_root.path().join("a.txt"), "old\n").unwrap();
    let decoded_input = json!({"input": patch.to_string()});
    let decoded_result = execute_tool_with(
        freeform_root.path(),
        "apply_patch",
        &decoded_input,
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!decoded_result.is_error, "{}", decoded_result.content);

    assert_eq!(
        fs::read_to_string(json_root.path().join("a.txt")).unwrap(),
        fs::read_to_string(freeform_root.path().join("a.txt")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(json_root.path().join("b.txt")).unwrap(),
        fs::read_to_string(freeform_root.path().join("b.txt")).unwrap()
    );
    let read = execute_tool_with(
        json_root.path(),
        "read",
        &json!({"path":"a.txt"}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(read.content.contains("new"));
}

#[test]
fn update_preserves_a_source_file_without_a_final_newline() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("no-final-lf.txt"), "old").unwrap();
    let patch = "*** Begin Patch\n*** Update File: no-final-lf.txt\n@@\n-old\n+new\n*** End Patch";
    let result = execute_tool_with(
        root.path(),
        "apply_patch",
        &json!({"input": patch}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        fs::read(root.path().join("no-final-lf.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn edit_schema_exposes_only_the_required_new_shape_while_legacy_parser_remains_usable() {
    let edit = tool_specs()
        .into_iter()
        .find(|tool| tool.name == "edit")
        .expect("edit tool");
    let required = edit.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|value| value == "path"));
    assert!(required.iter().any(|value| value == "edits"));
    assert!(edit.parameters["properties"].get("oldText").is_none());
    assert!(edit.parameters["properties"].get("newText").is_none());
    let item_required = edit.parameters["properties"]["edits"]["items"]["required"]
        .as_array()
        .unwrap();
    assert!(item_required.iter().any(|value| value == "oldText"));
    assert!(item_required.iter().any(|value| value == "newText"));

    let root = tempdir().unwrap();
    fs::write(root.path().join("legacy.txt"), "before\n").unwrap();
    let result = execute_tool_with(
        root.path(),
        "edit",
        &json!({"path":"legacy.txt", "oldText":"before", "newText":"after"}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        fs::read_to_string(root.path().join("legacy.txt")).unwrap(),
        "after\n"
    );
}

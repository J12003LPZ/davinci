//! Deterministic primary metric for the evidence-availability design:
//! a labeled needle must survive each reduction, or be recoverable from the
//! retained body. No model call and no network access.

use davinci_agent::codemode::projection::assemble_script_output;
use davinci_agent::ToolResult;
use davinci_coding_agent::native_extensions::{TokenGovernor, TokenGovernorConfig};
use serde_json::{json, Value};

fn ok(content: &str) -> ToolResult {
    ToolResult {
        content: content.into(),
        is_error: false,
        details: None,
    }
}

#[test]
fn diagnostic_view_byte_ratio_is_measured() {
    let dir = tempfile::tempdir().unwrap();
    let config = TokenGovernorConfig {
        compress_threshold_bytes: 1,
        compress_threshold_lines: 1,
        content_aware: true,
        store_dir: Some(dir.path().join("specialized")),
        ..Default::default()
    };
    let mut governor = TokenGovernor::new("diagnostic-ratio", config);
    let items = (0..120)
        .map(|i| {
            if i % 2 == 0 {
                json!({"id": i, "status": "error", "message": "failure detail"})
            } else {
                json!({"id": i, "status": "ok", "payload": "repetitive payload"})
            }
        })
        .collect::<Vec<_>>();
    let array = Value::Array(items).to_string();
    let result = governor.after_tool("bash", &json!({"command": "printf json"}), ok(&array));
    let original_bytes = array.len();
    let diagnostic_bytes = result.content.len();
    let ratio = diagnostic_bytes as f64 / original_bytes as f64;
    assert!(
        diagnostic_bytes < original_bytes,
        "diagnostic view grew to {ratio:.3} of the original"
    );

    let mut generic = TokenGovernor::new(
        "generic-ratio",
        TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            content_aware: false,
            store_dir: Some(dir.path().join("generic")),
            ..Default::default()
        },
    );
    let generic = generic.after_tool("bash", &json!({"command": "printf json"}), ok(&array));
    eprintln!(
        "evidence-availability bytes: diagnostic={diagnostic_bytes} generic={} original={original_bytes} ratio={ratio:.3}",
        generic.content.len()
    );
}

#[test]
fn needles_survive_the_governor_and_codemode() {
    let dir = tempfile::tempdir().unwrap();
    let config = TokenGovernorConfig {
        compress_threshold_bytes: 1,
        compress_threshold_lines: 1,
        content_aware: true,
        store_dir: Some(dir.path().join("governor")),
        ..Default::default()
    };
    let mut governor = TokenGovernor::new("evidence-availability", config);
    let array = serde_json::Value::Array(
        (0..30)
            .map(|i| {
                if i == 25 {
                    json!({"id": i, "status": "error", "message": "NEEDLE-GOVERNOR"})
                } else {
                    json!({"id": i, "status": "ok", "payload": "repetitive payload"})
                }
            })
            .collect(),
    )
    .to_string();
    let result = governor.after_tool("bash", &json!({"command": "printf json"}), ok(&array));
    assert!(
        result.content.contains("NEEDLE-GOVERNOR"),
        "governor needle lost: {}",
        result.content
    );
    assert_eq!(
        result.details.as_ref().unwrap()["tokenGovernor"]["strategy"],
        "specialized"
    );
    let output_id = result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
        .as_str()
        .unwrap()
        .to_string();
    let recovered = governor.retrieve(&json!({"id": output_id})).unwrap();
    assert!(
        recovered.content.contains("NEEDLE-GOVERNOR"),
        "first recovery lost the governor needle"
    );
    let governor_recovery_tokens = recovered.content.len().div_ceil(4);
    assert!(governor_recovery_tokens > 0);

    let body = (0..400)
        .map(|i| {
            if i == 249 {
                format!("NEEDLE-CODE{}\n", "x".repeat(88))
            } else {
                format!("{:0>99}\n", i)
            }
        })
        .collect::<String>();
    let items = vec![json!({"type": "text", "text": body})];
    let output = assemble_script_output(&items, 16 * 1024, 1024 * 1024).unwrap();
    assert!(
        !output.text.contains("NEEDLE-CODE"),
        "needle should be outside the visible head/tail: {}",
        output.text
    );
    assert!(
        output
            .retained_body
            .as_deref()
            .unwrap()
            .contains("NEEDLE-CODE"),
        "codemode retained body lost the needle"
    );
    assert_eq!(output.omitted_start_line, Some(110));
    let start = output.omitted_start_line.unwrap();
    let recovered = output
        .retained_body
        .as_deref()
        .unwrap()
        .lines()
        .skip(start - 1)
        .take(200)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        recovered.contains("NEEDLE-CODE"),
        "first read from the retained body lost the needle"
    );
    let codemode_recovery_tokens = recovered.len().div_ceil(4);
    assert!(codemode_recovery_tokens > 0);
}

use serde_json::json;

pub const TOOL_NAMES: &[&str] = &[
    "workspace_checkpoint",
    "workspace_diff",
    "workspace_restore",
];

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    let (description, parameters) = match name {
        "workspace_checkpoint" => (
            "Capture a bounded, owner-scoped workspace checkpoint without creating a Git commit or changing the index.",
            json!({
                "type": "object",
                "properties": {
                    "paths": {"type": "array", "items": {"type": "string", "minLength": 1, "maxLength": 4096}, "maxItems": 128},
                    "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "transactionId": {"type": "string", "minLength": 1, "maxLength": 128},
                    "label": {"type": "string", "maxLength": 256}
                },
                "additionalProperties": false
            }),
        ),
        "workspace_diff" => (
            "Compare a stored workspace checkpoint with the current scoped files and report exact changes and restore safety.",
            json!({
                "type": "object",
                "required": ["checkpointId"],
                "properties": {
                    "checkpointId": {"type": "string", "minLength": 1, "maxLength": 128},
                    "paths": {"type": "array", "items": {"type": "string", "minLength": 1, "maxLength": 4096}, "maxItems": 128}
                },
                "additionalProperties": false
            }),
        ),
        "workspace_restore" => (
            "Preview and safely restore only checkpoint-owned files whose current postimages are unchanged; newer or unrelated edits remain conflicts.",
            json!({
                "type": "object",
                "required": ["checkpointId"],
                "properties": {
                    "checkpointId": {"type": "string", "minLength": 1, "maxLength": 128},
                    "transactionId": {"type": "string", "minLength": 1, "maxLength": 128}
                },
                "additionalProperties": false
            }),
        ),
        _ => return None,
    };
    Some(davinci_ai::ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        constrained_sampling: None,
    })
}

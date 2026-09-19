use serde_json::json;

pub const TOOL_NAMES: &[&str] = &["verification_plan"];

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    (name == "verification_plan").then(|| davinci_ai::ToolSpec {
        name: name.to_string(),
        description: "Build a deterministic, source-bound verification sequence ordered from the cheapest reliable checks to broader required checks. Planning only: no command, browser, test, build, or CI action is executed.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "maxItems": 128,
                    "description": "Workspace-relative changed files"
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4096,
                    "description": "One workspace-relative changed file"
                },
                "transactionId": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "Host-owned transaction whose affected files should be planned"
                },
                "changedSymbols": {
                    "type": "array",
                    "items": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "maxItems": 128
                },
                "userRequirements": {
                    "type": "array",
                    "items": {"type": "string", "minLength": 1, "maxLength": 1024},
                    "maxItems": 32,
                    "description": "Explicit completion requirements supplied by the host or user"
                },
                "forceFull": {"type": "boolean"},
                "ciRequired": {"type": "boolean"}
            },
            "additionalProperties": false
        }),
        constrained_sampling: None,
    })
}

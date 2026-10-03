use serde_json::json;
pub const TOOL_NAMES: &[&str] = &["test_related", "test_impacted", "test_plan"];

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    if !TOOL_NAMES.contains(&name) {
        return None;
    }
    Some(davinci_ai::ToolSpec {
        name:name.into(),
        description:"Plan the first relevant TS/JS tests for changed paths or symbol IDs using repository dependencies and package scripts. Returns explained, bounded selection and required broader verification. Does not run tests or establish successful verification.".into(),
        parameters:json!({"type":"object","additionalProperties":false,"properties":{
            "paths":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":4096}},
            "path":{"type":"string","maxLength":4096},
            "symbolIds":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":4096}},
            "limit":{"type":"integer","minimum":1,"maximum":100},
            "refresh":{"type":"boolean","description":"Force full source-content reconciliation before final verification; normal planning uses observed changes."}
        }}),
        constrained_sampling:None,
    })
}

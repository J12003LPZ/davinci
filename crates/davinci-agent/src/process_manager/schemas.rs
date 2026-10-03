use crate::tools::AgentTool;
use serde_json::json;

pub fn tool_specs() -> Vec<AgentTool> {
    let id = json!({"type":"integer", "minimum":1, "maximum":4294967295u64});
    let lifetime = json!({"type":"string", "format":"uuid", "description":"Host-issued process lifetime; controls with an old lifetime are rejected."});
    let start = json!({
        "executable":{"type":"string"},
        "argv":{"type":"array","items":{"type":"string"},"maxItems":128},
        "cwd":{"type":"string"},
        "env":{"type":"object","additionalProperties":{"type":"string"},"maxProperties":32},
        "restart":{
            "type":"object","additionalProperties":false,
            "description":"Optional nonzero-exit retries, disabled by default. Every retry requires continuing permission; one-call approval is not reused.",
            "properties":{
                "max_restarts":{"type":"integer","minimum":0,"maximum":3},
                "backoff_ms":{"type":"integer","minimum":50,"maximum":5000}
            },"required":["max_restarts","backoff_ms"]
        },
        "ports":{"type":"array","items":{"type":"integer","minimum":1,"maximum":65535},"maxItems":8,"uniqueItems":true,"description":"Declared ports only; these are not verified listeners."}
    });
    [
        ("process_start", "Start or reuse a session-owned process using literal executable and argv. It survives this call; final owner release or host loss stops descendants. Environment inherits only platform essentials; explicit overrides need this call's permission. No shell string is executed.", start, vec!["executable"]),
        ("process_status", "Inspect state, lifetime identity, exit status and environment policy of an owned process. Does not start a process.", json!({"id":id,"lifetime":lifetime}), vec!["id"]),
        ("process_output", "Read a bounded page from an owned process's output ring. Cursors count raw bytes; invalid UTF-8 is replaced. Truncated means earlier bytes were dropped. Use next_cursor for more; this does not wait for process exit.", json!({"id":id,"cursor":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":65536}}), vec!["id"]),
        ("process_write", "Write literal UTF-8 text to an owned active process's stdin (16 KiB maximum). A timeout can mean partial delivery; inspect output before retrying; never replay a lost acknowledgement automatically.", json!({"id":id,"lifetime":lifetime,"text":{"type":"string","maxLength":16384}}), vec!["id","text"]),
        ("process_stop", "Release this owner's process lease. Final release requests descendant cleanup; inspect status until exited. Repeated release is safe; other owners retain their leases.", json!({"id":id,"lifetime":lifetime}), vec!["id"]),
        ("process_list", "List this owner's bounded managed process records. Does not discover other owners' processes or start anything.", json!({}), vec![]),
    ].into_iter().map(|(name, description, properties, required)| AgentTool { name:name.into(), description:description.into(), parameters:json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}) }).collect()
}

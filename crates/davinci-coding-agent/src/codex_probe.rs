//! Explicit maintainer probe for the ChatGPT Codex Responses backend.
//! Tests never call this module's network runner.

use davinci_ai::RawProviderReply;
use serde_json::{json, Value};

pub(crate) struct ProbeCase {
    pub name: &'static str,
    pub path_suffix: &'static str,
    pub body: Value,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ProbeOutcome {
    Accepted {
        cached_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
    },
    Rejected {
        status: u16,
        message: String,
    },
}

fn base(model_id: &str, input: Value) -> Value {
    json!({
        "model": model_id,
        "store": false,
        "stream": true,
        "instructions": "Reply with the single word OK.",
        "input": input,
    })
}

fn user(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": text}]
    })
}

/// `bootstrap` should be stable and long enough for the provider to report
/// a meaningful cache reuse result on the second identical request.
pub(crate) fn probe_cases(model_id: &str, bootstrap: &str) -> Vec<ProbeCase> {
    let read_tool = json!({
        "type": "function",
        "name": "read",
        "description": "Read a file.",
        "parameters": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }
    });

    let mut cases = Vec::new();
    cases.push(ProbeCase {
        name: "baseline",
        path_suffix: "",
        body: base(model_id, json!([user("hi")])),
    });

    let mut warm = base(model_id, json!([user(bootstrap), user("hi")]));
    warm["prompt_cache_key"] = json!("davinci-probe");
    cases.push(ProbeCase {
        name: "cache_warm",
        path_suffix: "",
        body: warm.clone(),
    });
    cases.push(ProbeCase {
        name: "cache_reuse",
        path_suffix: "",
        body: warm,
    });

    let mut options = base(model_id, json!([user("hi")]));
    options["prompt_cache_options"] = json!({"mode": "implicit", "ttl": "30m"});
    cases.push(ProbeCase {
        name: "prompt_cache_options",
        path_suffix: "",
        body: options,
    });

    let mut breakpoint = base(
        model_id,
        json!([
            {
                "type": "message",
                "role": "developer",
                "content": [{
                    "type": "input_text",
                    "text": bootstrap,
                    "prompt_cache_breakpoint": {"mode": "explicit"}
                }]
            },
            user("hi")
        ]),
    );
    breakpoint
        .as_object_mut()
        .expect("probe body is object")
        .remove("instructions");
    breakpoint["prompt_cache_options"] = json!({"mode": "implicit"});
    cases.push(ProbeCase {
        name: "explicit_breakpoint_without_instructions",
        path_suffix: "",
        body: breakpoint,
    });

    let mut prewarm = base(model_id, json!([user(bootstrap)]));
    prewarm["prompt_cache_options"] = json!({"prewarm": true});
    cases.push(ProbeCase {
        name: "prewarm",
        path_suffix: "",
        body: prewarm,
    });

    let mut allowed = base(model_id, json!([user("hi")]));
    allowed["tools"] = json!([read_tool.clone()]);
    allowed["tool_choice"] = json!({
        "type": "allowed_tools",
        "mode": "auto",
        "tools": [{"type": "function", "name": "read"}]
    });
    cases.push(ProbeCase {
        name: "allowed_tools",
        path_suffix: "",
        body: allowed,
    });

    let mut additional = base(
        model_id,
        json!([
            user("hi"),
            {"type": "additional_tools", "tools": [read_tool.clone()]}
        ]),
    );
    additional["tools"] = json!([]);
    cases.push(ProbeCase {
        name: "additional_tools",
        path_suffix: "",
        body: additional,
    });

    let mut custom = base(model_id, json!([user("hi")]));
    custom["tools"] = json!([{
        "type": "custom",
        "name": "apply_patch",
        "description": "Apply a patch.",
        "format": {
            "type": "grammar",
            "syntax": "lark",
            "definition": davinci_ai::APPLY_PATCH_LARK
        }
    }]);
    cases.push(ProbeCase {
        name: "custom_grammar_tool",
        path_suffix: "",
        body: custom,
    });

    let phase = base(
        model_id,
        json!([
            user("hi"),
            {
                "type": "message",
                "role": "assistant",
                "phase": "commentary",
                "content": [{"type": "output_text", "text": "Checking."}]
            },
            user("continue")
        ]),
    );
    cases.push(ProbeCase {
        name: "assistant_phase",
        path_suffix: "",
        body: phase,
    });

    let mut none = base(model_id, json!([user("hi")]));
    none["tools"] = json!([read_tool]);
    none["tool_choice"] = json!("none");
    cases.push(ProbeCase {
        name: "tool_choice_none",
        path_suffix: "",
        body: none,
    });

    cases.push(ProbeCase {
        name: "remote_compact",
        path_suffix: "/compact",
        body: json!({
            "model": model_id,
            "instructions": "Reply with OK.",
            "input": [user("hi")]
        }),
    });

    cases
}

fn completed_usage(body: &str) -> Option<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .and_then(|event| event["response"].get("usage").cloned())
}

pub(crate) fn classify(reply: &RawProviderReply) -> ProbeOutcome {
    if !(200..300).contains(&reply.status) {
        return ProbeOutcome::Rejected {
            status: reply.status,
            message: reply.body.chars().take(400).collect(),
        };
    }
    let usage = completed_usage(&reply.body);
    let detail = |key: &str| {
        usage
            .as_ref()
            .and_then(|usage| usage["input_tokens_details"][key].as_u64())
    };
    ProbeOutcome::Accepted {
        cached_tokens: detail("cached_tokens"),
        cache_write_tokens: detail("cache_write_tokens"),
    }
}

pub(crate) fn run(parsed: &crate::Args, report_path: &std::path::Path) -> Result<(), String> {
    let model_id = parsed
        .model
        .clone()
        .unwrap_or_else(|| "gpt-5.6-luna".into());
    let (model, auth) = crate::resolve_model_and_auth(parsed, "openai-codex", &model_id)?;
    let url = davinci_ai::request_url(&model, &auth);
    let bootstrap = "DaVinci probe stable text. ".repeat(260);
    let mut rows = Vec::new();

    for case in probe_cases(&model.id, &bootstrap) {
        let reply = davinci_ai::raw_provider_post(
            &model,
            &auth,
            &format!("{url}{}", case.path_suffix),
            &case.body,
        );
        let row = match reply {
            Ok(reply) => {
                let codex_headers: Vec<_> = reply
                    .headers
                    .iter()
                    .filter(|(name, _)| name.to_ascii_lowercase().starts_with("x-codex"))
                    .cloned()
                    .collect();
                json!({
                    "case": case.name,
                    "outcome": format!("{:?}", classify(&reply)),
                    "codexHeaders": codex_headers
                })
            }
            Err(error) => json!({
                "case": case.name,
                "outcome": format!("transport error: {error}")
            }),
        };
        eprintln!("{row}");
        rows.push(row);
    }

    std::fs::write(
        report_path,
        serde_json::to_string_pretty(&rows).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> RawProviderReply {
        RawProviderReply {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    #[test]
    fn completed_stream_reports_cache_usage() {
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1300,\"input_tokens_details\":{\"cached_tokens\":1280,\"cache_write_tokens\":0}}}}\n";
        assert_eq!(
            classify(&reply(200, body)),
            ProbeOutcome::Accepted {
                cached_tokens: Some(1280),
                cache_write_tokens: Some(0)
            }
        );
    }

    #[test]
    fn error_status_is_rejection_with_message() {
        let outcome = classify(&reply(
            400,
            "{\"detail\":\"Unsupported parameter: prompt_cache_options\"}",
        ));
        assert!(matches!(
            outcome,
            ProbeOutcome::Rejected {
                status: 400,
                ref message
            } if message.contains("prompt_cache_options")
        ));
    }

    #[test]
    fn every_case_has_unique_name() {
        let cases = probe_cases("gpt-5.6-luna", "x");
        let mut names: Vec<_> = cases.iter().map(|case| case.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), cases.len());
    }
}

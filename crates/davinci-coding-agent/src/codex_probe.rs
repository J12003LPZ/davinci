//! Explicit maintainer probe for the ChatGPT Codex Responses backend.
//! Tests never call this module's network runner.

use davinci_ai::{Model, RawProviderReply, ResolvedAuth};
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

    let mut freeform = base(
        model_id,
        json!([user(&format!("case=custom-freeform {bootstrap}"))]),
    );
    freeform["tools"] = json!([{
        "type": "custom",
        "name": "codemode",
        "description": "Run JavaScript source in a sandbox to orchestrate tools. The input is the JavaScript source itself, not JSON."
    }]);
    cases.push(ProbeCase {
        name: "custom_freeform_non_patch",
        path_suffix: "",
        body: freeform.clone(),
    });
    cases.push(ProbeCase {
        name: "custom_freeform_non_patch_reuse",
        path_suffix: "",
        body: freeform,
    });

    let mut programmatic = base(
        model_id,
        json!([user(&format!("case=programmatic {bootstrap}"))]),
    );
    programmatic["tools"] = json!([
        {
            "type": "function",
            "name": "read",
            "description": "Read a UTF-8 file and return its contents.",
            "parameters": {
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "additionalProperties": false
            },
            "output_schema": {
                "type": "object",
                "properties": {"content": {"type": "string"}},
                "required": ["content"],
                "additionalProperties": false
            },
            "allowed_callers": ["programmatic"]
        },
        {"type": "programmatic_tool_calling"}
    ]);
    cases.push(ProbeCase {
        name: "programmatic_tool_calling",
        path_suffix: "",
        body: programmatic.clone(),
    });
    cases.push(ProbeCase {
        name: "programmatic_tool_calling_reuse",
        path_suffix: "",
        body: programmatic,
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

fn reuse_base(case: &str) -> Option<&'static str> {
    match case {
        "cache_reuse" => Some("cache_warm"),
        "custom_freeform_non_patch_reuse" => Some("custom_freeform_non_patch"),
        "programmatic_tool_calling_reuse" => Some("programmatic_tool_calling"),
        _ => None,
    }
}

fn probe_row(case: &ProbeCase, reply: &Result<RawProviderReply, String>) -> Value {
    let mut row = match reply {
        Ok(reply) => {
            let codex_headers: Vec<_> = reply
                .headers
                .iter()
                .filter(|(name, _)| name.to_ascii_lowercase().starts_with("x-codex"))
                .cloned()
                .collect();
            let outcome = classify(reply);
            let (accepted, cached_tokens, cache_write_tokens) = match &outcome {
                ProbeOutcome::Accepted {
                    cached_tokens,
                    cache_write_tokens,
                } => (true, *cached_tokens, *cache_write_tokens),
                ProbeOutcome::Rejected { .. } => (false, None, None),
            };
            json!({
                "case": case.name,
                "accepted": accepted,
                "status": reply.status,
                "cached_tokens": cached_tokens,
                "cache_write_tokens": cache_write_tokens,
                "outcome": format!("{outcome:?}"),
                "codexHeaders": codex_headers
            })
        }
        Err(error) => json!({
            "case": case.name,
            "accepted": false,
            "status": null,
            "cached_tokens": null,
            "cache_write_tokens": null,
            "outcome": format!("transport error: {error}")
        }),
    };
    if let Some(base) = reuse_base(case.name) {
        row["reuses"] = json!(base);
    }
    row
}

fn select_case_names(
    all: &[&'static str],
    requested: &[String],
) -> Result<Vec<&'static str>, String> {
    if requested.is_empty() {
        return Ok(all.to_vec());
    }
    let mut selected = Vec::new();
    for name in requested {
        let Some(known) = all
            .iter()
            .copied()
            .find(|candidate| *candidate == name.as_str())
        else {
            return Err(format!(
                "unknown --codex-probe-case \"{name}\"; valid cases: {}",
                all.join(", ")
            ));
        };
        if !selected.contains(&known) {
            selected.push(known);
        }
    }
    Ok(selected)
}

/// The probe must follow the same route as a live turn: a ChatGPT backend login
/// moves the model to the backend, and every other credential keeps the
/// documented public route. Without this the probe sent the subscription token
/// to `api.openai.com` and recorded a 401 instead of a capability result.
fn probe_target(model: &Model, auth: &ResolvedAuth) -> (Model, String) {
    let routed = davinci_ai::codex_backend::route_model(model, auth).into_owned();
    let url = davinci_ai::request_url(&routed, auth);
    (routed, url)
}

pub(crate) fn run(parsed: &crate::Args, report_path: &std::path::Path) -> Result<(), String> {
    let model_id = parsed
        .model
        .clone()
        .unwrap_or_else(|| "gpt-5.6-luna".into());
    let (model, auth) = crate::resolve_model_and_auth(parsed, "openai-codex", &model_id)?;
    let (model, url) = probe_target(&model, &auth);
    let bootstrap = "DaVinci probe stable text. ".repeat(260);
    let cases = probe_cases(&model.id, &bootstrap);
    let names: Vec<&'static str> = cases.iter().map(|case| case.name).collect();
    let selected = select_case_names(&names, &parsed.codex_probe_case)?;
    let mut rows = Vec::new();

    for case in selected
        .iter()
        .filter_map(|name| cases.iter().find(|case| case.name == *name))
    {
        let reply = davinci_ai::raw_provider_post(
            &model,
            &auth,
            &format!("{url}{}", case.path_suffix),
            &case.body,
        );
        let row = probe_row(case, &reply);
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

    fn codex_model() -> Model {
        serde_json::from_value(json!({
            "id":"gpt-5.6-luna", "name":"fixture", "api":"openai-codex-responses",
            "provider":"openai-codex", "baseUrl":"https://api.openai.com/v1", "reasoning":true,
            "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
            "contextWindow":1000,"maxTokens":100
        }))
        .expect("model fixture")
    }

    fn auth(source: &str) -> ResolvedAuth {
        ResolvedAuth {
            api_key: Some("tok".into()),
            headers: std::collections::HashMap::from([(
                "Authorization".to_string(),
                "Bearer tok".to_string(),
            )]),
            source: source.into(),
        }
    }

    #[test]
    fn probe_follows_the_same_route_as_a_live_turn() {
        let model = codex_model();
        let (routed, url) = probe_target(&model, &auth(davinci_ai::codex_backend::AUTH_SOURCE));
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
        assert_eq!(
            routed.base_url.as_deref(),
            Some(davinci_ai::codex_backend::BACKEND_BASE_URL)
        );

        let (kept, public) = probe_target(&model, &auth("api-key"));
        assert_eq!(public, "https://api.openai.com/v1/responses");
        assert_eq!(kept.base_url.as_deref(), Some("https://api.openai.com/v1"));
    }

    fn find_case(name: &str) -> ProbeCase {
        probe_cases("gpt-5.6-luna", "x")
            .into_iter()
            .find(|case| case.name == name)
            .expect("probe case exists")
    }

    #[test]
    fn custom_freeform_case_is_a_non_patch_freeform_custom_tool() {
        let case = find_case("custom_freeform_non_patch");
        let tool = &case.body["tools"][0];
        assert_eq!(tool["type"], "custom");
        assert_eq!(tool["name"], "codemode");
        assert_ne!(tool["name"], "apply_patch");
        assert!(tool.get("format").is_none());
        assert!(tool.get("parameters").is_none());
        assert!(tool.get("strict").is_none());
    }

    #[test]
    fn base_pairs_have_diverging_prefixes() {
        let freeform = find_case("custom_freeform_non_patch");
        let programmatic = find_case("programmatic_tool_calling");
        assert_ne!(
            serde_json::to_vec(&freeform.body).unwrap(),
            serde_json::to_vec(&programmatic.body).unwrap()
        );
        let first_text = |case: &ProbeCase| {
            case.body["input"][0]["content"][0]["text"]
                .as_str()
                .expect("first input text")
                .to_string()
        };
        let (a, b) = (first_text(&freeform), first_text(&programmatic));
        assert!(!a.starts_with(&b));
        assert!(!b.starts_with(&a));
    }

    #[test]
    fn programmatic_tool_calling_case_sends_hosted_tool_and_allowed_callers() {
        let case = find_case("programmatic_tool_calling");
        let tools = case.body["tools"].as_array().expect("tools array");
        assert!(tools
            .iter()
            .any(|tool| tool["type"] == "programmatic_tool_calling"));
        let function = tools
            .iter()
            .find(|tool| tool["type"] == "function")
            .expect("function tool");
        assert_eq!(function["allowed_callers"], json!(["programmatic"]));
        assert!(function.get("parameters").is_some());
        assert!(function.get("output_schema").is_some());
        assert_eq!(function["parameters"]["additionalProperties"], false);
    }

    #[test]
    fn reuse_cases_repeat_their_base_body() {
        for (base_name, reuse_name) in [
            ("cache_warm", "cache_reuse"),
            (
                "custom_freeform_non_patch",
                "custom_freeform_non_patch_reuse",
            ),
            (
                "programmatic_tool_calling",
                "programmatic_tool_calling_reuse",
            ),
        ] {
            let base = find_case(base_name);
            let reuse = find_case(reuse_name);
            assert_eq!(
                serde_json::to_vec(&base.body).unwrap(),
                serde_json::to_vec(&reuse.body).unwrap()
            );
            assert_eq!(reuse_base(reuse_name), Some(base_name));
            let row = probe_row(&reuse, &Err("offline".into()));
            assert_eq!(row["reuses"], base_name);
        }
    }

    #[test]
    fn row_separates_accepted_status_and_cached_tokens() {
        let case = find_case("cache_reuse");
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1300,\"input_tokens_details\":{\"cached_tokens\":1280,\"cache_write_tokens\":0}}}}\n";
        let accepted = probe_row(&case, &Ok(reply(200, body)));
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["status"], 200);
        assert_eq!(accepted["cached_tokens"], 1280);
        assert_eq!(accepted["cache_write_tokens"], 0);

        let rejected = probe_row(
            &case,
            &Ok(reply(400, "{\"detail\":\"Unsupported parameter\"}")),
        );
        assert_eq!(rejected["accepted"], false);
        assert_eq!(rejected["status"], 400);
        assert!(rejected["cached_tokens"].is_null());

        let transport = probe_row(&case, &Err("connection reset".into()));
        assert_eq!(transport["accepted"], false);
        assert!(transport["status"].is_null());
    }

    #[test]
    fn case_filter_runs_only_named_cases_in_order() {
        let all: Vec<&'static str> = probe_cases("gpt-5.6-luna", "x")
            .iter()
            .map(|case| case.name)
            .collect();
        let requested = [
            "programmatic_tool_calling".to_string(),
            "custom_freeform_non_patch".to_string(),
        ];
        assert_eq!(
            select_case_names(&all, &requested).unwrap(),
            vec!["programmatic_tool_calling", "custom_freeform_non_patch"]
        );
        let repeated = ["baseline".to_string(), "baseline".to_string()];
        assert_eq!(
            select_case_names(&all, &repeated).unwrap(),
            vec!["baseline"]
        );
        assert_eq!(select_case_names(&all, &[]).unwrap(), all);

        let error = select_case_names(&all, &["nope".to_string()]).unwrap_err();
        assert!(error.contains("nope"));
        assert!(error.contains("baseline"));
    }
}

//! Model resolution matching `vendor/pi/packages/coding-agent/src/core/model-resolver.ts`.

use pi_ai::Model;
use pi_protocol::ThinkingLevel;

use crate::args::is_valid_thinking_level;

/// Default model IDs for each known provider (TS `defaultModelPerProvider`).
pub const DEFAULT_MODEL_PER_PROVIDER: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-4-6-v1"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-4-8"),
    ("openai", "gpt-5.5"),
    ("azure-openai-responses", "gpt-5.4"),
    ("openai-codex", "gpt-5.5"),
    ("radius", "auto"),
    ("nvidia", "nvidia/nemotron-3-super-120b-a12b"),
    ("deepseek", "deepseek-v4-pro"),
    ("google", "gemini-3.1-pro-preview"),
    ("google-vertex", "gemini-3.1-pro-preview"),
    ("github-copilot", "gpt-5.4"),
    ("openrouter", "moonshotai/kimi-k2.6"),
    ("vercel-ai-gateway", "zai/glm-5.1"),
    ("xai", "grok-4.6"),
    ("groq", "openai/gpt-oss-120b"),
    ("cerebras", "gpt-oss-120b"),
    ("zai", "glm-5.3"),
    ("zai-coding-cn", "glm-5.3"),
    ("mistral", "devstral-medium-latest"),
    ("minimax", "MiniMax-M2.7"),
    ("minimax-cn", "MiniMax-M2.7"),
    ("moonshotai", "kimi-k2.6"),
    ("moonshotai-cn", "kimi-k2.6"),
    ("huggingface", "moonshotai/Kimi-K2.6"),
    ("fireworks", "accounts/fireworks/models/kimi-k2p6"),
    ("together", "moonshotai/Kimi-K2.6"),
    ("baseten", "zai-org/GLM-5.2"),
    ("opencode", "kimi-k2.6"),
    ("opencode-go", "kimi-k2.6"),
    ("kimi-coding", "kimi-for-coding"),
    ("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.6"),
    ("cloudflare-ai-gateway", "workers-ai/@cf/moonshotai/kimi-k2.6"),
    ("qwen-token-plan", "qwen3.7-max"),
    ("qwen-token-plan-cn", "qwen3.7-max"),
    ("qwen-token-plan-individual", "qwen3.8-max"),
    ("xiaomi", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-cn", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-ams", "mimo-v2.5-pro"),
    ("xiaomi-token-plan-sgp", "mimo-v2.5-pro"),
];

#[derive(Debug, Clone)]
pub struct ScopedModelRef {
    pub model: Model,
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelScopeDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub pattern: String,
}

#[derive(Debug, Clone)]
pub struct ResolveModelScopeResult {
    pub scoped_models: Vec<ScopedModelRef>,
    pub diagnostics: Vec<ModelScopeDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct ParsedModelResult {
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolveCliModelResult {
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

pub fn models_are_equal(left: &Model, right: &Model) -> bool {
    left.provider == right.provider && left.id == right.id
}

fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }
    let bytes = id.as_bytes();
    if bytes.len() < 9 {
        return true;
    }
    let suffix = &bytes[bytes.len() - 9..];
    if suffix[0] != b'-' {
        return true;
    }
    !suffix[1..].iter().all(u8::is_ascii_digit)
}

pub fn find_exact_model_reference_match<'a>(
    model_reference: &str,
    available_models: &'a [Model],
) -> Option<&'a Model> {
    let trimmed = model_reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.to_ascii_lowercase();

    let canonical: Vec<&Model> = available_models
        .iter()
        .filter(|model| {
            format!("{}/{}", model.provider, model.id).to_ascii_lowercase() == normalized
        })
        .collect();
    if canonical.len() == 1 {
        return Some(canonical[0]);
    }
    if canonical.len() > 1 {
        return None;
    }

    if let Some(slash) = trimmed.find('/') {
        let provider = trimmed[..slash].trim();
        let model_id = trimmed[slash + 1..].trim();
        if !provider.is_empty() && !model_id.is_empty() {
            let provider_matches: Vec<&Model> = available_models
                .iter()
                .filter(|model| {
                    model.provider.eq_ignore_ascii_case(provider)
                        && model.id.eq_ignore_ascii_case(model_id)
                })
                .collect();
            if provider_matches.len() == 1 {
                return Some(provider_matches[0]);
            }
            if provider_matches.len() > 1 {
                return None;
            }
        }
    }

    let id_matches: Vec<&Model> = available_models
        .iter()
        .filter(|model| model.id.to_ascii_lowercase() == normalized)
        .collect();
    if id_matches.len() == 1 {
        Some(id_matches[0])
    } else {
        None
    }
}

fn try_match_model(model_pattern: &str, available_models: &[Model]) -> Option<Model> {
    if let Some(exact) = find_exact_model_reference_match(model_pattern, available_models) {
        return Some(exact.clone());
    }
    let needle = model_pattern.to_ascii_lowercase();
    let matches: Vec<&Model> = available_models
        .iter()
        .filter(|model| {
            model.id.to_ascii_lowercase().contains(&needle)
                || model.name.to_ascii_lowercase().contains(&needle)
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    let mut aliases: Vec<&Model> = matches
        .iter()
        .copied()
        .filter(|model| is_alias(&model.id))
        .collect();
    if !aliases.is_empty() {
        aliases.sort_by(|a, b| b.id.cmp(&a.id));
        return Some(aliases[0].clone());
    }
    let mut dated: Vec<&Model> = matches
        .iter()
        .copied()
        .filter(|model| !is_alias(&model.id))
        .collect();
    dated.sort_by(|a, b| b.id.cmp(&a.id));
    dated.first().map(|model| (*model).clone())
}

fn build_fallback_model(provider: &str, model_id: &str, available_models: &[Model]) -> Option<Model> {
    let provider_models: Vec<&Model> = available_models
        .iter()
        .filter(|model| model.provider == provider)
        .collect();
    let base = DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find(|(name, _)| *name == provider)
        .and_then(|(_, default_id)| {
            provider_models
                .iter()
                .find(|model| model.id == *default_id)
                .copied()
        })
        .or_else(|| provider_models.first().copied())?;
    let mut model = base.clone();
    model.id = model_id.to_string();
    model.name = model_id.to_string();
    Some(model)
}

pub fn parse_model_pattern(
    pattern: &str,
    available_models: &[Model],
    allow_invalid_thinking_level_fallback: bool,
) -> ParsedModelResult {
    if let Some(exact) = try_match_model(pattern, available_models) {
        return ParsedModelResult {
            model: Some(exact),
            thinking_level: None,
            warning: None,
        };
    }
    let Some(last_colon) = pattern.rfind(':') else {
        return ParsedModelResult {
            model: None,
            thinking_level: None,
            warning: None,
        };
    };
    let prefix = &pattern[..last_colon];
    let suffix = &pattern[last_colon + 1..];
    if let Some(level) = is_valid_thinking_level(suffix) {
        let result = parse_model_pattern(prefix, available_models, allow_invalid_thinking_level_fallback);
        if result.model.is_some() {
            return ParsedModelResult {
                thinking_level: if result.warning.is_some() {
                    None
                } else {
                    Some(level)
                },
                model: result.model,
                warning: result.warning,
            };
        }
        return result;
    }
    if !allow_invalid_thinking_level_fallback {
        return ParsedModelResult {
            model: None,
            thinking_level: None,
            warning: None,
        };
    }
    let result = parse_model_pattern(prefix, available_models, allow_invalid_thinking_level_fallback);
    if result.model.is_some() {
        return ParsedModelResult {
            model: result.model,
            thinking_level: None,
            warning: Some(format!(
                "Invalid thinking level \"{suffix}\" in pattern \"{pattern}\". Using default instead."
            )),
        };
    }
    result
}

pub fn glob_match(pattern: &str, value: &str) -> bool {
    glob_match_inner(
        &pattern.to_ascii_lowercase().into_bytes(),
        &value.to_ascii_lowercase().into_bytes(),
    )
}

fn glob_match_inner(pat: &[u8], val: &[u8]) -> bool {
    let mut pi = 0;
    let mut vi = 0;
    while pi < pat.len() {
        match pat[pi] {
            b'*' => {
                while pi < pat.len() && pat[pi] == b'*' {
                    pi += 1;
                }
                if pi == pat.len() {
                    return true;
                }
                while vi <= val.len() {
                    if glob_match_inner(&pat[pi..], &val[vi..]) {
                        return true;
                    }
                    if vi == val.len() {
                        return false;
                    }
                    vi += 1;
                }
                return false;
            }
            b'?' => {
                if vi >= val.len() {
                    return false;
                }
                pi += 1;
                vi += 1;
            }
            b'[' => match match_class(&pat[pi..], val.get(vi).copied()) {
                Some(next) => {
                    pi += next;
                    vi += 1;
                }
                None => return false,
            },
            c => {
                if vi >= val.len() || val[vi] != c {
                    return false;
                }
                pi += 1;
                vi += 1;
            }
        }
    }
    vi == val.len()
}

fn match_class(pat: &[u8], value: Option<u8>) -> Option<usize> {
    let Some(end) = pat.iter().skip(1).position(|ch| *ch == b']').map(|pos| pos + 1) else {
        return (value == Some(b'[')).then_some(1);
    };
    let Some(ch) = value else {
        return None;
    };
    let mut inner = &pat[1..end];
    let negate = matches!(inner.first(), Some(b'!' | b'^'));
    if negate {
        inner = &inner[1..];
    }
    let mut matched = false;
    let mut i = 0;
    while i < inner.len() {
        if i + 2 < inner.len() && inner[i + 1] == b'-' {
            let start = inner[i];
            let stop = inner[i + 2];
            if ch >= start && ch <= stop {
                matched = true;
                break;
            }
            i += 3;
            continue;
        }
        if inner[i] == ch {
            matched = true;
            break;
        }
        i += 1;
    }
    if matched ^ negate {
        Some(end + 1)
    } else {
        None
    }
}

pub fn model_pattern_matches(pattern: &str, full_id: &str, model_id: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        return glob_match(pattern, full_id) || glob_match(pattern, model_id);
    }
    full_id.eq_ignore_ascii_case(pattern) || model_id.eq_ignore_ascii_case(pattern)
}

pub fn resolve_model_scope_from_models(
    patterns: &[String],
    models: &[Model],
) -> ResolveModelScopeResult {
    let mut scoped_models = Vec::new();
    let mut diagnostics = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            let colon = pattern.rfind(':');
            let mut glob_pattern = pattern.as_str();
            let mut thinking_level = None;
            if let Some(idx) = colon {
                let suffix = &pattern[idx + 1..];
                if let Some(level) = is_valid_thinking_level(suffix) {
                    thinking_level = Some(level);
                    glob_pattern = &pattern[..idx];
                }
            }
            if let Some(exact) = find_exact_model_reference_match(glob_pattern, models) {
                if !scoped_models
                    .iter()
                    .any(|existing: &ScopedModelRef| models_are_equal(&existing.model, exact))
                {
                    scoped_models.push(ScopedModelRef {
                        model: exact.clone(),
                        thinking_level,
                    });
                }
                continue;
            }
            let matching: Vec<&Model> = models
                .iter()
                .filter(|model| {
                    let full = format!("{}/{}", model.provider, model.id);
                    glob_match(glob_pattern, &full) || glob_match(glob_pattern, &model.id)
                })
                .collect();
            if matching.is_empty() {
                diagnostics.push(ModelScopeDiagnostic {
                    code: "no-match",
                    message: format!("No models match pattern \"{pattern}\""),
                    pattern: pattern.clone(),
                });
                continue;
            }
            for model in matching {
                if !scoped_models
                    .iter()
                    .any(|existing| models_are_equal(&existing.model, model))
                {
                    scoped_models.push(ScopedModelRef {
                        model: model.clone(),
                        thinking_level,
                    });
                }
            }
            continue;
        }

        let parsed = parse_model_pattern(pattern, models, true);
        if let Some(warning) = parsed.warning.clone() {
            diagnostics.push(ModelScopeDiagnostic {
                code: "invalid-thinking-level",
                message: warning,
                pattern: pattern.clone(),
            });
        }
        let Some(model) = parsed.model else {
            diagnostics.push(ModelScopeDiagnostic {
                code: "no-match",
                message: format!("No models match pattern \"{pattern}\""),
                pattern: pattern.clone(),
            });
            continue;
        };
        if !scoped_models
            .iter()
            .any(|existing| models_are_equal(&existing.model, &model))
        {
            scoped_models.push(ScopedModelRef {
                model,
                thinking_level: parsed.thinking_level,
            });
        }
    }
    ResolveModelScopeResult {
        scoped_models,
        diagnostics,
    }
}

pub fn resolve_cli_model(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    cli_thinking: Option<ThinkingLevel>,
    models: &[Model],
    has_configured_auth: impl Fn(&str) -> bool,
) -> ResolveCliModelResult {
    let Some(cli_model) = cli_model else {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: None,
        };
    };
    if models.is_empty() {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(
                "No models available. Check your installation or add models to models.json."
                    .into(),
            ),
        };
    }

    let mut provider_map = std::collections::BTreeMap::new();
    for model in models {
        provider_map.insert(model.provider.to_ascii_lowercase(), model.provider.clone());
    }

    let mut provider = cli_provider.and_then(|name| provider_map.get(&name.to_ascii_lowercase()).cloned());
    if cli_provider.is_some() && provider.is_none() {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(format!(
                "Unknown provider \"{}\". Use --list-models to see available providers/models.",
                cli_provider.unwrap_or_default()
            )),
        };
    }

    let mut pattern = cli_model.to_string();
    let mut inferred_provider = false;
    if provider.is_none() {
        if let Some(slash) = cli_model.find('/') {
            let maybe_provider = &cli_model[..slash];
            if let Some(canonical) = provider_map.get(&maybe_provider.to_ascii_lowercase()) {
                provider = Some(canonical.clone());
                pattern = cli_model[slash + 1..].to_string();
                inferred_provider = true;
            }
        }
    }

    if provider.is_none() {
        let lower = cli_model.to_ascii_lowercase();
        let exact_matches: Vec<&Model> = models
            .iter()
            .filter(|model| {
                model.id.to_ascii_lowercase() == lower
                    || format!("{}/{}", model.provider, model.id).to_ascii_lowercase() == lower
            })
            .collect();
        if exact_matches.len() == 1 {
            return ResolveCliModelResult {
                model: Some(exact_matches[0].clone()),
                thinking_level: None,
                warning: None,
                error: None,
            };
        }
        if exact_matches.len() > 1 {
            let authenticated: Vec<&Model> = exact_matches
                .iter()
                .copied()
                .filter(|model| has_configured_auth(&model.provider))
                .collect();
            if authenticated.len() == 1 {
                return ResolveCliModelResult {
                    model: Some(authenticated[0].clone()),
                    thinking_level: None,
                    warning: None,
                    error: None,
                };
            }
            let mut matches: Vec<String> = exact_matches
                .iter()
                .map(|model| format!("{}/{}", model.provider, model.id))
                .collect();
            matches.sort();
            let auth_hint = if authenticated.is_empty() {
                "No matching provider is authenticated."
            } else {
                "More than one matching provider is authenticated."
            };
            return ResolveCliModelResult {
                model: None,
                thinking_level: None,
                warning: None,
                error: Some(format!(
                    "Model \"{cli_model}\" is ambiguous across providers: {}. {auth_hint} Use --provider or provider/model.",
                    matches.join(", ")
                )),
            };
        }
    }

    if let (Some(cli_provider), Some(provider)) = (cli_provider, provider.as_deref()) {
        let prefix = format!("{provider}/");
        if cli_model.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
            pattern = cli_model[prefix.len()..].to_string();
        }
        let _ = cli_provider;
    }

    let candidates: Vec<Model> = if let Some(provider) = provider.as_deref() {
        models
            .iter()
            .filter(|model| model.provider == provider)
            .cloned()
            .collect()
    } else {
        models.to_vec()
    };
    let parsed = parse_model_pattern(&pattern, &candidates, false);
    if let Some(model) = parsed.model.clone() {
        if inferred_provider && !has_configured_auth(&model.provider) {
            let raw_exact: Vec<&Model> = models
                .iter()
                .filter(|item| {
                    item.id.eq_ignore_ascii_case(cli_model) && !models_are_equal(item, &model)
                })
                .collect();
            if !raw_exact.is_empty() {
                let authenticated: Vec<&Model> = raw_exact
                    .iter()
                    .copied()
                    .filter(|item| has_configured_auth(&item.provider))
                    .collect();
                if authenticated.len() == 1 {
                    return ResolveCliModelResult {
                        model: Some(authenticated[0].clone()),
                        thinking_level: None,
                        warning: None,
                        error: None,
                    };
                }
            }
        }
        return ResolveCliModelResult {
            model: parsed.model,
            thinking_level: parsed.thinking_level,
            warning: parsed.warning,
            error: None,
        };
    }

    if inferred_provider {
        let lower = cli_model.to_ascii_lowercase();
        if let Some(exact) = models.iter().find(|model| {
            model.id.to_ascii_lowercase() == lower
                || format!("{}/{}", model.provider, model.id).to_ascii_lowercase() == lower
        }) {
            return ResolveCliModelResult {
                model: Some(exact.clone()),
                thinking_level: None,
                warning: None,
                error: None,
            };
        }
        let fallback = parse_model_pattern(cli_model, models, false);
        if fallback.model.is_some() {
            return ResolveCliModelResult {
                model: fallback.model,
                thinking_level: fallback.thinking_level,
                warning: fallback.warning,
                error: None,
            };
        }
    }

    if let Some(provider) = provider.as_deref() {
        let mut fallback_pattern = pattern.as_str();
        let mut fallback_thinking = None;
        if cli_thinking.is_none() {
            if let Some(last_colon) = pattern.rfind(':') {
                let suffix = &pattern[last_colon + 1..];
                if let Some(level) = is_valid_thinking_level(suffix) {
                    fallback_pattern = &pattern[..last_colon];
                    fallback_thinking = Some(level);
                }
            }
        }
        if let Some(mut fallback_model) = build_fallback_model(provider, fallback_pattern, models) {
            let requested = cli_thinking.or(fallback_thinking);
            if requested.is_some_and(|level| level != ThinkingLevel::Off) {
                fallback_model.reasoning = true;
            }
            let fallback_warning = match parsed.warning {
                Some(warning) => format!(
                    "{warning} Model \"{fallback_pattern}\" not found for provider \"{provider}\". Using custom model id."
                ),
                None => format!(
                    "Model \"{fallback_pattern}\" not found for provider \"{provider}\". Using custom model id."
                ),
            };
            return ResolveCliModelResult {
                model: Some(fallback_model),
                thinking_level: fallback_thinking,
                warning: Some(fallback_warning),
                error: None,
            };
        }
    }

    let display = provider
        .as_deref()
        .map(|name| format!("{name}/{pattern}"))
        .unwrap_or_else(|| cli_model.to_string());
    ResolveCliModelResult {
        model: None,
        thinking_level: None,
        warning: parsed.warning,
        error: Some(format!(
            "Model \"{display}\" not found. Use --list-models to see available models."
        )),
    }
}

pub fn thinking_level_for_model_switch(
    explicit: Option<ThinkingLevel>,
    per_model: Option<ThinkingLevel>,
    default_level: Option<ThinkingLevel>,
    current: ThinkingLevel,
) -> ThinkingLevel {
    explicit
        .or(per_model)
        .or(default_level)
        .unwrap_or(current)
}

#[cfg(test)]
fn mock_model(provider: &str, id: &str, name: &str, reasoning: bool) -> Model {
    use pi_ai::ModelCost;
    Model {
        id: id.into(),
        name: name.into(),
        api: "anthropic-messages".into(),
        provider: provider.into(),
        base_url: None,
        reasoning,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 1.0,
            output: 2.0,
            cache_read: 0.1,
            cache_write: 1.0,
        },
        context_window: 128_000,
        max_tokens: 8192,
        compat: serde_json::Value::Null,
        headers: Default::default(),
        thinking_level_map: Default::default(),
    }
}

#[cfg(test)]
fn fixture_models() -> Vec<Model> {
    vec![
        mock_model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5", true),
        mock_model("openai", "gpt-4o", "GPT-4o", false),
        mock_model(
            "openrouter",
            "qwen/qwen3-coder:exacto",
            "Qwen3 Coder Exacto",
            true,
        ),
        mock_model(
            "openrouter",
            "openai/gpt-4o:extended",
            "GPT-4o Extended",
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pattern_fuzzy_and_thinking_suffix() {
        let models = fixture_models();
        let exact = parse_model_pattern("claude-sonnet-4-5", &models, true);
        assert_eq!(exact.model.as_ref().unwrap().id, "claude-sonnet-4-5");
        assert!(exact.thinking_level.is_none());

        let fuzzy = parse_model_pattern("sonnet", &models, true);
        assert_eq!(fuzzy.model.as_ref().unwrap().id, "claude-sonnet-4-5");

        let thinking = parse_model_pattern("sonnet:high", &models, true);
        assert_eq!(thinking.model.as_ref().unwrap().id, "claude-sonnet-4-5");
        assert_eq!(thinking.thinking_level, Some(ThinkingLevel::High));

        let invalid = parse_model_pattern("sonnet:random", &models, true);
        assert_eq!(invalid.model.as_ref().unwrap().id, "claude-sonnet-4-5");
        assert!(invalid.thinking_level.is_none());
        assert!(invalid
            .warning
            .as_deref()
            .unwrap()
            .contains("Invalid thinking level"));

        let colon_id = parse_model_pattern("qwen/qwen3-coder:exacto:high", &models, true);
        assert_eq!(colon_id.model.as_ref().unwrap().id, "qwen/qwen3-coder:exacto");
        assert_eq!(colon_id.thinking_level, Some(ThinkingLevel::High));
    }

    #[test]
    fn resolve_cli_model_patterns() {
        let models = fixture_models();
        let slash = resolve_cli_model(None, Some("openai/gpt-4o"), None, &models, |_| false);
        assert_eq!(slash.model.as_ref().unwrap().provider, "openai");
        assert_eq!(slash.model.as_ref().unwrap().id, "gpt-4o");

        let fuzzy = resolve_cli_model(Some("openai"), Some("4o"), None, &models, |_| false);
        assert_eq!(fuzzy.model.as_ref().unwrap().id, "gpt-4o");

        let thinking = resolve_cli_model(None, Some("sonnet:high"), None, &models, |_| false);
        assert_eq!(thinking.model.as_ref().unwrap().id, "claude-sonnet-4-5");
        assert_eq!(thinking.thinking_level, Some(ThinkingLevel::High));

        let openrouter = resolve_cli_model(
            None,
            Some("openai/gpt-4o:extended"),
            None,
            &models,
            |_| false,
        );
        assert_eq!(openrouter.model.as_ref().unwrap().provider, "openrouter");
        assert_eq!(openrouter.model.as_ref().unwrap().id, "openai/gpt-4o:extended");

        let strict = resolve_cli_model(
            Some("openai"),
            Some("gpt-4o:extended"),
            None,
            &models,
            |_| false,
        );
        assert_eq!(strict.model.as_ref().unwrap().provider, "openai");
        assert_eq!(strict.model.as_ref().unwrap().id, "gpt-4o:extended");
        assert!(strict.warning.as_deref().unwrap().contains("custom model id"));

        let empty = resolve_cli_model(Some("openai"), Some("gpt-4o"), None, &[], |_| false);
        assert!(empty
            .error
            .as_deref()
            .unwrap()
            .contains("No models available"));
    }

    #[test]
    fn resolve_cli_model_ambiguous_prefers_authenticated() {
        let models = vec![
            mock_model("azure-openai-responses", "gpt-5.6-sol", "GPT 5.6 Sol", false),
            mock_model("openai-codex", "gpt-5.6-sol", "GPT 5.6 Sol", false),
        ];
        let resolved = resolve_cli_model(None, Some("gpt-5.6-sol"), None, &models, |provider| {
            provider == "openai-codex"
        });
        assert_eq!(resolved.model.as_ref().unwrap().provider, "openai-codex");

        let ambiguous = resolve_cli_model(None, Some("gpt-5.6-sol"), None, &models, |_| false);
        assert!(ambiguous
            .error
            .as_deref()
            .unwrap()
            .contains("ambiguous across providers"));
    }

    #[test]
    fn resolve_scope_globs_and_thinking() {
        let models = fixture_models();
        let result = resolve_model_scope_from_models(
            &[
                "sonnet:high".into(),
                "gpt-4o:invalid".into(),
                "missing".into(),
            ],
            &models,
        );
        assert_eq!(result.scoped_models.len(), 2);
        assert_eq!(result.scoped_models[0].model.id, "claude-sonnet-4-5");
        assert_eq!(
            result.scoped_models[0].thinking_level,
            Some(ThinkingLevel::High)
        );
        assert!(result.scoped_models[1].thinking_level.is_none());
        assert_eq!(result.diagnostics[0].code, "invalid-thinking-level");
        assert_eq!(result.diagnostics[1].code, "no-match");

        let glob = resolve_model_scope_from_models(&["*sonnet*".into()], &models);
        assert_eq!(glob.scoped_models[0].model.id, "claude-sonnet-4-5");

        let provider_glob = resolve_model_scope_from_models(&["openai/*".into()], &models);
        assert_eq!(provider_glob.scoped_models[0].model.id, "gpt-4o");
        assert!(model_pattern_matches("*sonnet*", "anthropic/claude-sonnet-4-5", "claude-sonnet-4-5"));
        assert!(!model_pattern_matches("openai/*", "anthropic/claude-sonnet-4-5", "claude-sonnet-4-5"));
    }

    #[test]
    fn bracketed_id_is_exact_before_glob() {
        let mut models = fixture_models();
        models.push(mock_model(
            "custom",
            "bracketed-model[1m]",
            "Bracketed Model",
            true,
        ));
        let result =
            resolve_model_scope_from_models(&["custom/bracketed-model[1m]:high".into()], &models);
        assert_eq!(result.scoped_models[0].model.id, "bracketed-model[1m]");
        assert_eq!(
            result.scoped_models[0].thinking_level,
            Some(ThinkingLevel::High)
        );
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn alias_wins_over_dated() {
        let models = vec![
            mock_model("anthropic", "claude-sonnet-4-5-20250929", "Dated", true),
            mock_model("anthropic", "claude-sonnet-4-5", "Alias", true),
        ];
        let result = parse_model_pattern("sonnet", &models, true);
        assert_eq!(result.model.as_ref().unwrap().id, "claude-sonnet-4-5");
    }

    #[test]
    fn thinking_switch_priority() {
        assert_eq!(
            thinking_level_for_model_switch(
                Some(ThinkingLevel::High),
                Some(ThinkingLevel::Low),
                Some(ThinkingLevel::Medium),
                ThinkingLevel::Off,
            ),
            ThinkingLevel::High
        );
        assert_eq!(
            thinking_level_for_model_switch(
                None,
                Some(ThinkingLevel::Low),
                Some(ThinkingLevel::Medium),
                ThinkingLevel::Off,
            ),
            ThinkingLevel::Low
        );
        assert_eq!(
            thinking_level_for_model_switch(
                None,
                None,
                Some(ThinkingLevel::Medium),
                ThinkingLevel::Off,
            ),
            ThinkingLevel::Medium
        );
        assert_eq!(
            thinking_level_for_model_switch(None, None, None, ThinkingLevel::Xhigh),
            ThinkingLevel::Xhigh
        );
    }
}

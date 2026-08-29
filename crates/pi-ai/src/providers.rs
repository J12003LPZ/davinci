use crate::catalog::{flatten_catalog, load_builtin_models, Model};

#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub api: &'static str,
    pub env_vars: &'static [&'static str],
    pub oauth: bool,
    pub oauth_name: Option<&'static str>,
}

pub const PROVIDER_SPECS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "amazon-bedrock",
        name: "Amazon Bedrock",
        base_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
        api: "bedrock-converse-stream",
        env_vars: &[
            "AWS_ACCESS_KEY_ID",
            "AWS_BEARER_TOKEN_BEDROCK",
            "AWS_PROFILE",
        ],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "ant-ling",
        name: "Ant Ling",
        base_url: "https://api.antling.com/v1",
        api: "openai-completions",
        env_vars: &["ANT_LING_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com",
        api: "anthropic-messages",
        env_vars: &[
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
        ],
        oauth: true,
        oauth_name: Some("Anthropic (Claude Pro/Max)"),
    },
    ProviderSpec {
        id: "google",
        name: "Google",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        api: "google-generative-ai",
        env_vars: &["GEMINI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "google-vertex",
        name: "Google Vertex",
        base_url: "https://aiplatform.googleapis.com",
        api: "google-vertex",
        env_vars: &["GOOGLE_CLOUD_API_KEY", "GOOGLE_CLOUD_PROJECT"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        api: "openai-responses",
        env_vars: &["OPENAI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "azure-openai-responses",
        name: "Azure OpenAI",
        base_url: "https://openai.azure.com",
        api: "azure-openai-responses",
        env_vars: &["AZURE_OPENAI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "openai-codex",
        name: "OpenAI Codex",
        base_url: "https://chatgpt.com/backend-api",
        api: "openai-codex-responses",
        env_vars: &["OPENAI_API_KEY"],
        oauth: true,
        oauth_name: Some("ChatGPT Codex"),
    },
    ProviderSpec {
        id: "radius",
        name: "Radius",
        base_url: "https://api.radius.dev",
        api: "openai-completions",
        env_vars: &["RADIUS_API_KEY"],
        oauth: true,
        oauth_name: Some("Radius"),
    },
    ProviderSpec {
        id: "nvidia",
        name: "NVIDIA",
        base_url: "https://integrate.api.nvidia.com/v1",
        api: "openai-completions",
        env_vars: &["NVIDIA_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com",
        api: "openai-completions",
        env_vars: &["DEEPSEEK_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "github-copilot",
        name: "GitHub Copilot",
        base_url: "https://api.githubcopilot.com",
        api: "openai-completions",
        env_vars: &["COPILOT_GITHUB_TOKEN"],
        oauth: true,
        oauth_name: Some("GitHub Copilot"),
    },
    ProviderSpec {
        id: "xai",
        name: "xAI",
        base_url: "https://api.x.ai/v1",
        api: "openai-completions",
        env_vars: &["XAI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        api: "openai-completions",
        env_vars: &["GROQ_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "cerebras",
        name: "Cerebras",
        base_url: "https://api.cerebras.ai/v1",
        api: "openai-completions",
        env_vars: &["CEREBRAS_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api: "openai-completions",
        env_vars: &["OPENROUTER_API_KEY"],
        oauth: true,
        oauth_name: Some("OpenRouter"),
    },
    ProviderSpec {
        id: "vercel-ai-gateway",
        name: "Vercel AI Gateway",
        base_url: "https://ai-gateway.vercel.sh/v1",
        api: "openai-completions",
        env_vars: &["AI_GATEWAY_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "zai",
        name: "ZAI",
        base_url: "https://api.z.ai/api/coding/paas/v4",
        api: "openai-completions",
        env_vars: &["ZAI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "zai-coding-cn",
        name: "ZAI Coding CN",
        base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        api: "openai-completions",
        env_vars: &["ZAI_CODING_CN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        api: "mistral-conversations",
        env_vars: &["MISTRAL_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "minimax",
        name: "MiniMax",
        base_url: "https://api.minimax.io/anthropic",
        api: "anthropic-messages",
        env_vars: &["MINIMAX_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "minimax-cn",
        name: "MiniMax CN",
        base_url: "https://api.minimaxi.com/anthropic",
        api: "anthropic-messages",
        env_vars: &["MINIMAX_CN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "moonshotai",
        name: "Moonshot AI",
        base_url: "https://api.moonshot.ai/v1",
        api: "openai-completions",
        env_vars: &["MOONSHOT_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "moonshotai-cn",
        name: "Moonshot AI CN",
        base_url: "https://api.moonshot.cn/v1",
        api: "openai-completions",
        env_vars: &["MOONSHOT_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "huggingface",
        name: "Hugging Face",
        base_url: "https://router.huggingface.co/v1",
        api: "openai-completions",
        env_vars: &["HF_TOKEN"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "fireworks",
        name: "Fireworks",
        base_url: "https://api.fireworks.ai/inference/v1",
        api: "openai-completions",
        env_vars: &["FIREWORKS_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "together",
        name: "Together",
        base_url: "https://api.together.xyz/v1",
        api: "openai-completions",
        env_vars: &["TOGETHER_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "baseten",
        name: "Baseten",
        base_url: "https://inference.baseten.co/v1",
        api: "openai-completions",
        env_vars: &["BASETEN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "opencode",
        name: "OpenCode Zen",
        base_url: "https://opencode.ai/zen/v1",
        api: "openai-completions",
        env_vars: &["OPENCODE_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "opencode-go",
        name: "OpenCode Go",
        base_url: "https://opencode.ai/zen/go/v1",
        api: "openai-completions",
        env_vars: &["OPENCODE_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "kimi-coding",
        name: "Kimi For Coding",
        base_url: "https://api.kimi.com/coding/v1",
        api: "openai-completions",
        env_vars: &["KIMI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "cloudflare-workers-ai",
        name: "Cloudflare Workers AI",
        base_url: "https://api.cloudflare.com/client/v4/accounts",
        api: "openai-completions",
        env_vars: &["CLOUDFLARE_API_KEY", "CLOUDFLARE_ACCOUNT_ID"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "cloudflare-ai-gateway",
        name: "Cloudflare AI Gateway",
        base_url: "https://gateway.ai.cloudflare.com/v1",
        api: "openai-completions",
        env_vars: &[
            "CLOUDFLARE_API_KEY",
            "CLOUDFLARE_ACCOUNT_ID",
            "CLOUDFLARE_GATEWAY_ID",
        ],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "qwen-token-plan",
        name: "Qwen Token Plan",
        base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        api: "openai-completions",
        env_vars: &["QWEN_TOKEN_PLAN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "qwen-token-plan-cn",
        name: "Qwen Token Plan CN",
        base_url: "https://coding.dashscope.aliyuncs.com/v1",
        api: "openai-completions",
        env_vars: &["QWEN_TOKEN_PLAN_CN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "qwen-token-plan-individual",
        name: "Qwen Token Plan Individual",
        base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        api: "openai-completions",
        env_vars: &["QWEN_TOKEN_PLAN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "xiaomi",
        name: "Xiaomi MiMo",
        base_url: "https://api.xiaomimimo.com/v1",
        api: "openai-completions",
        env_vars: &["XIAOMI_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "xiaomi-token-plan-cn",
        name: "Xiaomi Token Plan CN",
        base_url: "https://api.xiaomimimo.com/cn/v1",
        api: "openai-completions",
        env_vars: &["XIAOMI_TOKEN_PLAN_CN_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "xiaomi-token-plan-ams",
        name: "Xiaomi Token Plan AMS",
        base_url: "https://api.xiaomimimo.com/ams/v1",
        api: "openai-completions",
        env_vars: &["XIAOMI_TOKEN_PLAN_AMS_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
    ProviderSpec {
        id: "xiaomi-token-plan-sgp",
        name: "Xiaomi Token Plan SGP",
        base_url: "https://api.xiaomimimo.com/sgp/v1",
        api: "openai-completions",
        env_vars: &["XIAOMI_TOKEN_PLAN_SGP_API_KEY"],
        oauth: false,
        oauth_name: None,
    },
];

#[derive(Debug, Clone)]
pub struct Provider {
    pub spec: ProviderSpec,
    pub models: Vec<Model>,
}

impl Provider {
    pub fn id(&self) -> &str {
        self.spec.id
    }
}

pub fn builtin_providers() -> Vec<Provider> {
    let models = load_builtin_models();
    PROVIDER_SPECS
        .iter()
        .map(|spec| Provider {
            spec: spec.clone(),
            models: models
                .iter()
                .filter(|model| model.provider == spec.id)
                .cloned()
                .collect(),
        })
        .collect()
}

pub fn provider_spec(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDER_SPECS.iter().find(|spec| spec.id == id)
}

pub fn load_models_json(path: &std::path::Path) -> Result<Vec<Model>, String> {
    let raw = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
    if let Some(providers) = value.get("providers").and_then(|v| v.as_object()) {
        let mut models = Vec::new();
        for (provider, catalog) in providers {
            models.extend(flatten_catalog(provider, catalog));
        }
        return Ok(models);
    }
    Ok(flatten_catalog("custom", &value))
}

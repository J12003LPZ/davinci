//! Library embed API matching `vendor/pi/packages/coding-agent/src/core/sdk.ts`.

use std::path::PathBuf;

use pi_agent::{
    default_system_prompt, discover_prompt_templates, discover_skills, expand_user_text,
    load_context_files, Agent, CompactionResult, BUILTIN_TOOLS,
};
use pi_ai::{
    find_model, load_builtin_models, snapshot_availability, AuthStorage, ModelConfig,
    ModelRuntimeSnapshot,
};
use pi_session::{default_agent_dir, latest_session, JsonlSession};

use crate::settings::load_merged_settings;

const DEFAULT_ACTIVE_TOOLS: &[&str] = &["read", "bash", "edit", "write"];
type AgentEventListener = Box<dyn Fn(&pi_agent::AgentEvent)>;

#[derive(Debug, Clone, Default)]
pub struct CreateAgentSessionOptions {
    pub cwd: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub thinking_level: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub exclude_tools: Option<Vec<String>>,
    /// `"all"` disables every tool; `"builtin"` disables built-in tools only.
    pub no_tools: Option<String>,
    pub session_dir: Option<PathBuf>,
    pub session_name: Option<String>,
    /// Resume the latest JSONL session for `cwd` (TS sessionManager restore).
    pub continue_session: bool,
    pub session_path: Option<PathBuf>,
    /// Models available for cycling (TS `scopedModels`).
    pub scoped_models: Option<Vec<String>>,
    /// Extra tool names to register (TS `customTools`).
    pub custom_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ExtensionLoadError {
    pub path: String,
    pub error: String,
}

/// TS `LoadExtensionsResult` (runtime is owned by the JS host; embed exposes loaded manifests).
#[derive(Debug, Clone, Default)]
pub struct LoadExtensionsResult {
    pub extensions: Vec<ExtensionManifest>,
    pub errors: Vec<ExtensionLoadError>,
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionManifest {
    pub name: String,
    pub path: Option<String>,
}

pub struct AgentSession {
    pub agent: Agent,
    pub cwd: PathBuf,
    pub agent_dir: PathBuf,
    pub scoped_models: Vec<String>,
    pub custom_tools: Vec<String>,
    pub model_runtime: ModelRuntimeSnapshot,
    listeners: Vec<AgentEventListener>,
}

impl AgentSession {
    pub fn subscribe(&mut self, listener: impl Fn(&pi_agent::AgentEvent) + 'static) {
        self.listeners.push(Box::new(listener));
    }

    pub fn run<F>(&mut self, complete: F) -> Result<Vec<pi_agent::AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<pi_ai::AssistantMessage, String>,
    {
        let events = self.agent.run_loop(complete)?;
        for event in &events {
            for listener in &self.listeners {
                listener(event);
            }
        }
        Ok(events)
    }

    pub fn prompt_and_run<F>(
        &mut self,
        text: &str,
        complete: F,
    ) -> Result<Vec<pi_agent::AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<pi_ai::AssistantMessage, String>,
    {
        self.prompt(text);
        self.run(complete)
    }
}

impl AgentSession {
    pub fn prompt(&mut self, text: &str) -> pi_ai::ChatMessage {
        self.prompt_with(text, &[])
    }

    pub fn prompt_with(
        &mut self,
        text: &str,
        images: &[pi_ai::MessageContent],
    ) -> pi_ai::ChatMessage {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.prompt_with(&expanded, images)
    }

    pub fn steer(&mut self, text: &str, images: Vec<pi_ai::MessageContent>) {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.queues.enqueue_steer_with(expanded, images);
    }

    pub fn follow_up(&mut self, text: &str, images: Vec<pi_ai::MessageContent>) {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.queues.enqueue_follow_up_with(expanded, images);
    }

    pub fn compact(&mut self, instructions: Option<&str>) -> CompactionResult {
        self.agent.compact(instructions)
    }

    pub fn abort(&mut self) {
        self.agent.abort();
    }
}

pub struct CreateAgentSessionResult {
    pub session: AgentSession,
    pub extensions_result: LoadExtensionsResult,
    pub model_fallback_message: Option<String>,
}

pub fn create_agent_session(
    options: CreateAgentSessionOptions,
) -> Result<CreateAgentSessionResult, String> {
    let cwd = options
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let agent_dir = options.agent_dir.clone().unwrap_or_else(default_agent_dir);
    let settings = load_merged_settings(&agent_dir, &cwd);
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = cwd.clone();
    agent.context_files = load_context_files(&cwd, true);
    agent.skills = discover_skills(&[cwd.join(".pi").join("skills"), agent_dir.join("skills")]);
    agent.templates =
        discover_prompt_templates(&[cwd.join(".pi").join("prompts"), agent_dir.join("prompts")]);

    let models = load_builtin_models();
    let mut model_fallback_message = None;
    if let (Some(provider), Some(model_id)) =
        (options.provider.as_deref(), options.model.as_deref())
    {
        if let Some(model) = find_model(&models, provider, model_id) {
            agent.provider = model.provider.clone();
            agent.model_id = model.id.clone();
        } else {
            model_fallback_message = Some(format!("Could not restore model {provider}/{model_id}"));
        }
    }
    if agent.model_id.is_empty() {
        if let Some(model) = models.first() {
            if let Some(existing) = model_fallback_message.take() {
                model_fallback_message =
                    Some(format!("{existing}. Using {}/{}", model.provider, model.id));
            }
            agent.provider = model.provider.clone();
            agent.model_id = model.id.clone();
        } else {
            model_fallback_message = Some(pi_ai::NO_MODELS_AVAILABLE.to_string());
        }
    }

    let thinking = options
        .thinking_level
        .clone()
        .or_else(|| settings.default_thinking_level.clone());
    if let Some(level) = thinking.as_deref() {
        agent.thinking_level = parse_thinking(level);
    }

    agent.tools = initial_tools(&options, settings.default_tools.as_deref());
    agent.tool_registry = BUILTIN_TOOLS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    agent.auto_compaction = settings.compaction_enabled();
    agent.compaction = settings.compaction_settings();
    agent.block_images = settings.block_images();
    agent.auto_resize_images = settings.image_auto_resize();

    let session_dir = options
        .session_dir
        .clone()
        .unwrap_or_else(pi_session::default_session_dir);

    if let Some(path) = &options.session_path {
        let store = JsonlSession::open(path).map_err(|err| err.to_string())?;
        agent.load_from_session(store);
    } else if options.continue_session {
        if let Some(summary) = latest_session(&session_dir, Some(&cwd.to_string_lossy()))
            .map_err(|err| err.to_string())?
        {
            let store = JsonlSession::open(&summary.path).map_err(|err| err.to_string())?;
            agent.load_from_session(store);
        }
    }
    if agent.session.is_none() {
        agent.session = Some(
            JsonlSession::create(
                &session_dir,
                &cwd.to_string_lossy(),
                options.session_name.as_deref(),
            )
            .map_err(|err| err.to_string())?,
        );
    }

    let scoped_models = options.scoped_models.clone().unwrap_or_default();
    let custom_tools = options.custom_tools.clone().unwrap_or_default();
    if !custom_tools.is_empty() {
        agent.apply_extension_tools(&custom_tools);
    }

    let model_runtime = embed_model_runtime(&agent_dir);
    let extensions_result = load_extensions_result(&agent_dir, &cwd, &settings.extensions);

    Ok(CreateAgentSessionResult {
        session: AgentSession {
            agent,
            cwd,
            agent_dir,
            scoped_models,
            custom_tools,
            model_runtime,
            listeners: Vec::new(),
        },
        extensions_result,
        model_fallback_message,
    })
}

fn embed_model_runtime(agent_dir: &std::path::Path) -> ModelRuntimeSnapshot {
    let models = load_builtin_models();
    let config = ModelConfig::load(&agent_dir.join("models.json"));
    let storage = AuthStorage::open(&agent_dir.join("auth.json"))
        .unwrap_or_else(|_| AuthStorage::in_memory());
    let env = std::env::vars().collect();
    snapshot_availability(models, &config, &storage, &env, Default::default(), None)
}

fn load_extensions_result(
    agent_dir: &std::path::Path,
    cwd: &std::path::Path,
    configured: &[String],
) -> LoadExtensionsResult {
    let mut names = configured.to_vec();
    for root in [
        agent_dir.join("extensions"),
        cwd.join(".pi").join("extensions"),
    ] {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                if let Some(name) = entry.file_name().to_str() {
                    if !names.iter().any(|existing| existing == name) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    let mut extensions = Vec::new();
    let mut errors = Vec::new();
    for name in names {
        let candidates = [
            agent_dir.join("extensions").join(&name),
            cwd.join(".pi").join("extensions").join(&name),
        ];
        let dir = candidates.iter().find(|path| path.is_dir());
        let Some(dir) = dir else {
            errors.push(ExtensionLoadError {
                path: name.clone(),
                error: format!("Extension not found: {name}"),
            });
            continue;
        };
        let manifest_path = if dir.join("pi.extension.json").exists() {
            dir.join("pi.extension.json")
        } else {
            dir.join("package.json")
        };
        if manifest_path.exists() {
            match std::fs::read_to_string(&manifest_path) {
                Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                    Ok(value) => extensions.push(ExtensionManifest {
                        name: value
                            .get("name")
                            .and_then(|item| item.as_str())
                            .unwrap_or(&name)
                            .to_string(),
                        path: Some(dir.display().to_string()),
                    }),
                    Err(error) => errors.push(ExtensionLoadError {
                        path: manifest_path.display().to_string(),
                        error: error.to_string(),
                    }),
                },
                Err(error) => errors.push(ExtensionLoadError {
                    path: manifest_path.display().to_string(),
                    error: error.to_string(),
                }),
            }
        } else {
            extensions.push(ExtensionManifest {
                name: name.clone(),
                path: Some(dir.display().to_string()),
            });
        }
    }
    LoadExtensionsResult { extensions, errors }
}

fn initial_tools(
    options: &CreateAgentSessionOptions,
    configured: Option<&[String]>,
) -> Vec<String> {
    let excluded = options
        .exclude_tools
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let names: Vec<String> = if let Some(tools) = &options.tools {
        tools.clone()
    } else if matches!(options.no_tools.as_deref(), Some("all") | Some("builtin")) {
        Vec::new()
    } else if let Some(tools) = configured {
        tools.to_vec()
    } else {
        DEFAULT_ACTIVE_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    };
    names
        .into_iter()
        .filter(|name| !excluded.contains(name))
        .collect()
}

fn parse_thinking(value: &str) -> pi_protocol::ThinkingLevel {
    pi_protocol::ThinkingLevel::parse(value).unwrap_or(pi_protocol::ThinkingLevel::Off)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_agent_session_applies_tools_and_prompt() {
        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            tools: Some(vec!["read".into(), "bash".into()]),
            exclude_tools: Some(vec!["bash".into()]),
            thinking_level: Some("high".into()),
            session_name: Some("sdk".into()),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert_eq!(result.session.agent.tools, vec!["read".to_string()]);
        assert!(!result.session.model_runtime.all.is_empty());
        assert!(result.extensions_result.errors.is_empty());
        assert_eq!(
            result.session.agent.thinking_level,
            pi_protocol::ThinkingLevel::High
        );
        assert!(result.session.agent.session.is_some());
        let message = result.session.agent.messages.len();
        let mut session = result.session;
        session.prompt("hello from sdk");
        assert_eq!(session.agent.messages.len(), message + 1);
        assert_eq!(session.agent.messages.last().unwrap().role, "user");
    }

    #[test]
    fn no_tools_all_starts_empty() {
        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            session_dir: Some(dir.path().join("sessions")),
            no_tools: Some("all".into()),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert!(result.session.agent.tools.is_empty());
    }

    #[test]
    fn prompt_expands_templates_and_continue_restores() {
        let dir = tempdir().unwrap();
        let prompts = dir.path().join(".pi").join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("review.md"), "Review this code: $1").unwrap();
        let session_dir = dir.path().join("sessions");
        let first = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(session_dir.clone()),
            session_name: Some("one".into()),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        let mut session = first.session;
        session.prompt("/review src/index.ts");
        let last = pi_ai::content_text(&session.agent.messages.last().unwrap().content);
        assert_eq!(last, "Review this code: src/index.ts");
        session.steer("/review extra.rs", Vec::new());
        assert_eq!(
            session
                .agent
                .queues
                .steer
                .last()
                .map(|item| item.text.as_str()),
            Some("Review this code: extra.rs")
        );

        let restored = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(session_dir),
            continue_session: true,
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert!(!restored.session.agent.messages.is_empty());
        assert_eq!(
            pi_ai::content_text(&restored.session.agent.messages.last().unwrap().content),
            "Review this code: src/index.ts"
        );
    }

    #[test]
    fn settings_default_tools_apply_when_unspecified() {
        let dir = tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("settings.json"),
            r#"{"defaultTools":["read"]}"#,
        )
        .unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(agent_dir),
            session_dir: Some(dir.path().join("sessions")),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert_eq!(result.session.agent.tools, vec!["read".to_string()]);
    }

    #[test]
    fn subscribe_and_custom_tools_match_embed_api() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            scoped_models: Some(vec!["google/gemini-3-flash".into()]),
            custom_tools: Some(vec!["ticket".into()]),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        let mut session = result.session;
        assert_eq!(session.scoped_models, vec!["google/gemini-3-flash"]);
        assert!(session.agent.tools.contains(&"ticket".to_string()));
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let kinds_clone = kinds.clone();
        session.subscribe(move |event| {
            kinds_clone.borrow_mut().push(event.kind().to_string());
        });
        session
            .prompt_and_run("hi", |_| {
                Ok(AssistantMessage {
                    id: "a1".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text { text: "ok".into() }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();
        let kinds = kinds.borrow().clone();
        assert!(kinds.contains(&"agent_start".to_string()));
        assert!(kinds.contains(&"message_update".to_string()));
        assert!(kinds.contains(&"agent_end".to_string()));
    }

    #[test]
    fn extensions_result_loads_project_manifest() {
        let dir = tempdir().unwrap();
        let ext = dir.path().join(".pi").join("extensions").join("demo");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("pi.extension.json"),
            r#"{"name":"demo","tools":[{"name":"ticket"}]}"#,
        )
        .unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert_eq!(result.extensions_result.extensions[0].name, "demo");
        assert!(
            result.session.model_runtime.get_error().is_none()
                || result.session.model_runtime.availability_error.is_none()
        );
    }
}

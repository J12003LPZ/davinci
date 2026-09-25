//! Library embed API matching `vendor/pi/packages/coding-agent/src/core/sdk.ts`.

use std::path::{Path, PathBuf};

use davinci_agent::{
    discover_prompt_templates, discover_skills, expand_user_text, load_context_files, Agent,
    CompactionResult, CustomToolExecutor, ToolError, BUILTIN_TOOLS,
};
use davinci_ai::{
    find_model, load_builtin_models, snapshot_availability, AuthStorage, ModelConfig,
    ModelRuntimeSnapshot,
};
use davinci_session::{default_agent_dir, latest_session, JsonlSession};

use crate::settings::load_merged_settings;

const DEFAULT_ACTIVE_TOOLS: &[&str] = &["read", "bash", "edit", "write"];
type AgentEventListener = Box<dyn Fn(&davinci_agent::AgentEvent)>;

#[derive(Debug, Clone, Default)]
pub struct CreateAgentSessionOptions {
    pub cwd: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub prompt_profile: Option<davinci_agent::PromptProfile>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
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

struct LanguageIntelligenceAttachment {
    manager: crate::native_extensions::language_intelligence::LanguageIntelligence,
    registration: davinci_agent::runtime::capabilities::CapabilityRegistration,
    previous_executor: Option<CustomToolExecutor>,
    previous_semantic: Option<std::sync::Arc<dyn davinci_agent::semantic::SemanticService>>,
    attached_executor: CustomToolExecutor,
    attached_semantic: std::sync::Arc<dyn davinci_agent::semantic::SemanticService>,
    tools: Vec<String>,
}

pub struct AgentSession {
    pub agent: Agent,
    pub cwd: PathBuf,
    pub agent_dir: PathBuf,
    pub scoped_models: Vec<String>,
    pub custom_tools: Vec<String>,
    pub model_runtime: ModelRuntimeSnapshot,
    listeners: Vec<AgentEventListener>,
    language_intelligence: Option<LanguageIntelligenceAttachment>,
    attachment_allowed_tools: Option<std::collections::HashSet<String>>,
    attachment_excluded_tools: std::collections::HashSet<String>,
    attachment_no_tools: Option<String>,
}

impl AgentSession {
    pub fn subscribe(&mut self, listener: impl Fn(&davinci_agent::AgentEvent) + 'static) {
        self.listeners.push(Box::new(listener));
    }

    pub fn run<F>(&mut self, complete: F) -> Result<Vec<davinci_agent::AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String>,
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
    ) -> Result<Vec<davinci_agent::AgentEvent>, String>
    where
        F: FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String>,
    {
        self.prompt(text);
        self.run(complete)
    }
}

impl AgentSession {
    pub fn prompt(&mut self, text: &str) -> davinci_ai::ChatMessage {
        self.prompt_with(text, &[])
    }

    pub fn prompt_with(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
    ) -> davinci_ai::ChatMessage {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.prompt_user_with(&expanded, images)
    }

    pub fn steer(&mut self, text: &str, images: Vec<davinci_ai::MessageContent>) {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.queues.enqueue_steer_with(expanded, images);
    }

    pub fn follow_up(&mut self, text: &str, images: Vec<davinci_ai::MessageContent>) {
        let expanded = expand_user_text(text, &self.agent.skills, &self.agent.templates);
        self.agent.queues.enqueue_follow_up_with(expanded, images);
    }

    pub fn compact(&mut self, instructions: Option<&str>) -> CompactionResult {
        self.agent.compact(instructions)
    }

    pub fn abort(&mut self) {
        self.agent.abort();
    }

    /// Opt in to the canonical native language-intelligence owner for this
    /// embedding session. The SDK never enables unrelated native extensions.
    pub fn attach_language_intelligence(
        &mut self,
        config: crate::native_extensions::language_intelligence::LanguageIntelligenceConfig,
        tools: &[String],
    ) -> Result<(), String> {
        if self.language_intelligence.is_some() {
            return Err("language_intelligence_already_attached".into());
        }
        let runtime = self.agent.runtime.clone().ok_or("sdk_runtime_required")?;
        if self.attachment_no_tools.as_deref() == Some("all") {
            return Err("language_intelligence_tools_excluded".into());
        }
        let mut selected = Vec::new();
        for tool in tools {
            if !crate::native_extensions::language_intelligence::TOOL_NAMES.contains(&tool.as_str()) {
                return Err(format!("unsupported language-intelligence tool: {tool}"));
            }
            if self.attachment_excluded_tools.contains(tool)
                || self
                    .attachment_allowed_tools
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(tool))
            {
                return Err(format!("language-intelligence tool excluded by session policy: {tool}"));
            }
            if !selected.contains(tool) {
                selected.push(tool.clone());
            }
        }

        let manager = crate::native_extensions::language_intelligence::LanguageIntelligence::new(
            &self.cwd,
            config,
        );
        manager.set_permissions(Some(self.agent.permissions.clone()));
        let facade: std::sync::Arc<dyn davinci_agent::semantic::SemanticService> =
            std::sync::Arc::new(crate::semantic::SemanticServiceFacade::local(manager.clone()));

        let capabilities = selected
            .iter()
            .filter_map(|name| crate::native_extensions::language_intelligence::tool_spec(name))
            .map(|spec| {
                davinci_agent::runtime::RuntimeCapability::new(
                    spec.name.clone(),
                    davinci_agent::runtime::CapabilitySource::NativeExtension,
                    davinci_agent::permission::ToolClass::Read,
                    true,
                    &spec.parameters,
                    Some(env!("CARGO_PKG_VERSION").to_string()),
                )
                .with_description(spec.description)
            })
            .collect::<Vec<_>>();
        let registration = runtime
            .capability_registry
            .register_owned(capabilities)
            .map_err(|error| error.to_string())?;

        let previous_executor = self.agent.custom_tool_executor.clone();
        let previous_semantic = self.agent.tool_context.semantic.clone();
        let manager_for_executor = manager.clone();
        let selected_for_executor = selected.clone();
        let previous_for_executor = previous_executor.clone();
        let attached_executor = CustomToolExecutor::new_with_context(
            move |cwd, name, args, context| {
                if selected_for_executor.iter().any(|tool| tool == name) {
                    let timeout = std::time::Duration::from_secs(60);
                    let budget = crate::native_extensions::language_intelligence::RequestBudget {
                        deadline: std::time::Instant::now() + timeout,
                        cancelled: context.abort.clone(),
                    };
                    return manager_for_executor.execute_with_budget(name, args, budget);
                }
                if let Some(previous) = &previous_for_executor {
                    return previous.execute_with_context(cwd, name, args, context);
                }
                Err(ToolError::Unknown(name.into()))
            },
        );

        {
            let mut authorized = self
                .agent
                .tool_context
                .authorized_tools
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for tool in &selected {
                authorized.insert(tool.clone());
            }
        }
        {
            let mut exposure = self
                .agent
                .tool_context
                .tool_exposure
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for tool in &selected {
                exposure.activate_authorized(tool, true);
            }
        }
        for tool in &selected {
            if !self.agent.tool_registry.contains(tool) {
                self.agent.tool_registry.push(tool.clone());
            }
            if !self.agent.tools.contains(tool) {
                self.agent.tools.push(tool.clone());
            }
        }
        self.agent.custom_tool_executor = Some(attached_executor.clone());
        self.agent.tool_context.semantic = Some(facade.clone());
        self.language_intelligence = Some(LanguageIntelligenceAttachment {
            manager,
            registration,
            previous_executor,
            previous_semantic,
            attached_executor,
            attached_semantic: facade,
            tools: selected,
        });
        Ok(())
    }

    pub fn detach_language_intelligence(&mut self) -> Result<(), String> {
        if self.agent.is_streaming {
            return Err("language_intelligence_detach_while_running".into());
        }
        let Some(attachment) = self.language_intelligence.take() else {
            return Ok(());
        };
        let runtime = self.agent.runtime.clone().ok_or("sdk_runtime_required")?;
        if !self
            .agent
            .custom_tool_executor
            .as_ref()
            .is_some_and(|executor| executor.same_instance(&attachment.attached_executor))
        {
            self.language_intelligence = Some(attachment);
            return Err("language_intelligence_executor_ownership_changed".into());
        }
        if !self
            .agent
            .tool_context
            .semantic
            .as_ref()
            .is_some_and(|semantic| std::sync::Arc::ptr_eq(semantic, &attachment.attached_semantic))
        {
            self.language_intelligence = Some(attachment);
            return Err("language_intelligence_semantic_ownership_changed".into());
        }
        runtime
            .capability_registry
            .unregister_owned(&attachment.registration)
            .map_err(|error| error.to_string())?;

        self.agent.custom_tool_executor = attachment.previous_executor;
        self.agent.tool_context.semantic = attachment.previous_semantic;
        self.agent
            .tool_registry
            .retain(|name| !attachment.tools.contains(name));
        self.agent.tools.retain(|name| !attachment.tools.contains(name));
        {
            let mut authorized = self
                .agent
                .tool_context
                .authorized_tools
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for tool in &attachment.tools {
                authorized.remove(tool);
            }
        }
        {
            let mut exposure = self
                .agent
                .tool_context
                .tool_exposure
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for tool in &attachment.tools {
                exposure.activate_authorized(tool, false);
            }
        }
        attachment.manager.shutdown();
        Ok(())
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        if let Some(attachment) = self.language_intelligence.take() {
            attachment.manager.shutdown();
        }
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
    let trusted = crate::trust::resolve_project_trusted(
        &agent_dir,
        &cwd,
        None,
        settings.default_project_trust.as_deref(),
        &settings.trusted_projects,
    );
    let env_profile = std::env::var("DAVINCI_PROMPT_PROFILE")
        .ok()
        .or_else(|| std::env::var("PI_PROMPT_PROFILE").ok());
    let resolved_profile = crate::prompt_host::resolve_prompt_profile(
        options.prompt_profile,
        settings.prompt_profile.as_deref(),
        env_profile.as_deref(),
    )?
    .profile;
    let mut agent = Agent::new_builtin(resolved_profile);
    agent.cwd = cwd.clone();
    agent.context_files = load_context_files(&cwd, true);
    let mut skill_roots = project_resource_roots(&cwd, trusted, "skills");
    skill_roots.push(agent_dir.join("skills"));
    agent.skills = discover_skills(&skill_roots);
    let mut prompt_roots = project_resource_roots(&cwd, trusted, "prompts");
    prompt_roots.push(agent_dir.join("prompts"));
    agent.templates = discover_prompt_templates(&prompt_roots);
    agent.permissions = std::sync::Arc::new(davinci_agent::PermissionState::new(
        sdk_permission_policy(&agent_dir, &cwd),
    ));

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
            model_fallback_message = Some(davinci_ai::NO_MODELS_AVAILABLE.to_string());
        }
    }

    let mut session = if let Some(custom) = &options.system_prompt {
        davinci_agent::PromptSessionState::custom(custom)
    } else {
        davinci_agent::PromptSessionState::builtin(resolved_profile)
    };
    for extra in &options.append_system_prompt {
        let text = if Path::new(extra).exists() {
            std::fs::read_to_string(extra).map_err(|err| err.to_string())?
        } else {
            extra.clone()
        };
        session.append(text);
    }
    let ctx = davinci_agent::prompt::PromptContext {
        provider: &agent.provider,
        model_id: &agent.model_id,
        permission_mode: agent.permission_mode(),
        plan_active: agent.is_plan_mode(),
    };
    let composed = session.render_and_record(&ctx);
    agent.system_prompt = composed.text;
    agent.prompt_manifest = Some(composed.manifest);
    agent.prompt_session = session;

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
        .unwrap_or_else(davinci_session::default_session_dir);

    if let Some(path) = &options.session_path {
        let store = JsonlSession::open(path).map_err(|err| err.to_string())?;
        agent.load_from_session(store)?;
    } else if options.continue_session {
        if let Some(summary) = latest_session(&session_dir, Some(&cwd.to_string_lossy()))
            .map_err(|err| err.to_string())?
        {
            let store = JsonlSession::open(&summary.path).map_err(|err| err.to_string())?;
            agent.load_from_session(store)?;
        }
    }
    if agent.session.is_none() {
        let store = JsonlSession::create(
            &session_dir,
            &cwd.to_string_lossy(),
            options.session_name.as_deref(),
        )
        .map_err(|err| err.to_string())?;
        agent.load_from_session(store)?;
    }

    let scoped_models = options.scoped_models.clone().unwrap_or_default();
    let custom_tools = options.custom_tools.clone().unwrap_or_default();
    if !custom_tools.is_empty() {
        agent.apply_extension_tools(&custom_tools);
    }

    let model_runtime = embed_model_runtime(&agent_dir);
    let extensions_result = load_extensions_result(&agent_dir, &cwd, &settings.extensions, trusted);

    Ok(CreateAgentSessionResult {
        session: AgentSession {
            agent,
            cwd,
            agent_dir,
            scoped_models,
            custom_tools,
            model_runtime,
            listeners: Vec::new(),
            language_intelligence: None,
            attachment_allowed_tools: options
                .tools
                .as_ref()
                .map(|tools| tools.iter().cloned().collect()),
            attachment_excluded_tools: options
                .exclude_tools
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            attachment_no_tools: options.no_tools.clone(),
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

/// Project resource directories are included only after project trust is
/// established, so untrusted files never enter SDK prompts or extensions.
fn project_resource_roots(cwd: &Path, trusted: bool, kind: &str) -> Vec<PathBuf> {
    if trusted {
        crate::project_config::all(cwd, kind)
    } else {
        Vec::new()
    }
}

/// SDK sessions keep their documented AlwaysApprove default while honoring
/// user deny rules and any deny rules from a trusted project.
fn sdk_permission_policy(agent_dir: &Path, cwd: &Path) -> davinci_agent::PermissionPolicy {
    let product = crate::permissions::policy_for(agent_dir, cwd, None, None);
    davinci_agent::PermissionPolicy {
        deny: product.deny,
        project_trusted: product.project_trusted,
        ..davinci_agent::PermissionPolicy::default()
    }
}

fn load_extensions_result(
    agent_dir: &std::path::Path,
    cwd: &std::path::Path,
    configured: &[String],
    trusted: bool,
) -> LoadExtensionsResult {
    let mut names = configured.to_vec();
    let mut roots = vec![agent_dir.join("extensions")];
    roots.extend(project_resource_roots(cwd, trusted, "extensions"));
    for root in &roots {
        if let Ok(entries) = std::fs::read_dir(root) {
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
        let candidates: Vec<PathBuf> = roots.iter().map(|root| root.join(&name)).collect();
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

fn parse_thinking(value: &str) -> davinci_protocol::ThinkingLevel {
    davinci_protocol::ThinkingLevel::parse(value).unwrap_or(davinci_protocol::ThinkingLevel::Off)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_agent_session_options_carry_prompt_configuration() {
        let options = CreateAgentSessionOptions {
            prompt_profile: Some(davinci_agent::PromptProfile::Preview),
            system_prompt: Some("custom replacement".into()),
            append_system_prompt: vec!["first append".into(), "second append".into()],
            ..CreateAgentSessionOptions::default()
        };

        assert_eq!(
            options.prompt_profile,
            Some(davinci_agent::PromptProfile::Preview)
        );
        assert_eq!(options.system_prompt.as_deref(), Some("custom replacement"));
        assert_eq!(
            options.append_system_prompt,
            vec!["first append".to_string(), "second append".to_string()]
        );
    }

    #[test]
    fn default_sdk_session_activates_stable_prompt_profile() {
        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();

        assert!(result.session.agent.prompt_session.is_builtin());
        assert_eq!(
            result.session.agent.prompt_session.profile(),
            Some(davinci_agent::PromptProfile::Stable)
        );
        let manifest = result
            .session
            .agent
            .prompt_manifest
            .as_ref()
            .expect("manifest must be present");
        assert_eq!(manifest.profile, "stable");
    }

    #[test]
    fn custom_replacement_sdk_session_receives_no_builtin_modules() {
        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            system_prompt: Some("You are a specialized auditor.".into()),
            append_system_prompt: vec!["extra instruction".into()],
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();

        assert!(result.session.agent.prompt_session.is_custom());
        let manifest = result
            .session
            .agent
            .prompt_manifest
            .as_ref()
            .expect("manifest must be present");
        assert_eq!(manifest.profile, "custom");
        assert!(manifest.modules.is_empty());
        assert!(result
            .session
            .agent
            .system_prompt
            .contains("specialized auditor"));
        assert!(result
            .session
            .agent
            .system_prompt
            .contains("extra instruction"));
    }

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
            davinci_protocol::ThinkingLevel::High
        );
        assert!(result.session.agent.session.is_some());
        assert!(result.session.agent.runtime_for_session().is_some());
        assert!(result
            .session
            .agent
            .session
            .as_ref()
            .unwrap()
            .path
            .with_extension("tasks.jsonl")
            .is_file());
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
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("trust.json"),
            format!(
                "{{{:?}: true}}\n",
                crate::trust::canonicalize_trust_path(dir.path())
            ),
        )
        .unwrap();
        let prompts = dir.path().join(".pi").join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("review.md"), "Review this code: $1").unwrap();
        let session_dir = dir.path().join("sessions");
        let first = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(agent_dir.clone()),
            session_dir: Some(session_dir.clone()),
            session_name: Some("one".into()),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        let mut session = first.session;
        session.prompt("/review src/index.ts");
        let last = davinci_ai::content_text(&session.agent.messages.last().unwrap().content);
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

        // A cold resume must release the original session's single-writer lease.
        drop(session);
        let restored = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(agent_dir),
            session_dir: Some(session_dir),
            continue_session: true,
            ..CreateAgentSessionOptions::default()
        })
        .unwrap();
        assert!(!restored.session.agent.messages.is_empty());
        assert_eq!(
            davinci_ai::content_text(&restored.session.agent.messages.last().unwrap().content),
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
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
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
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("trust.json"),
            format!(
                "{{{:?}: true}}\n",
                crate::trust::canonicalize_trust_path(dir.path())
            ),
        )
        .unwrap();
        let ext = dir.path().join(".pi").join("extensions").join("demo");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("pi.extension.json"),
            r#"{"name":"demo","tools":[{"name":"ticket"}]}"#,
        )
        .unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(agent_dir),
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

    #[test]
    fn untrusted_project_skills_prompts_and_extensions_are_not_loaded() {
        let dir = tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let project = dir.path().join("project");
        let skill = project.join(".pi").join("skills").join("planted");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: planted\ndescription: x\n---\n# x\n",
        )
        .unwrap();
        std::fs::create_dir_all(project.join(".pi").join("extensions").join("evil")).unwrap();
        let result = load_extensions_result(&agent_dir, &project, &[], false);
        assert!(result.extensions.is_empty());
        assert!(result.errors.is_empty());
        let skills = project_resource_roots(&project, false, "skills");
        assert!(skills.is_empty());
        assert_eq!(
            project_resource_roots(&project, true, "skills"),
            vec![project.join(".pi").join("skills")]
        );
    }

    #[test]
    fn user_deny_rules_apply_to_sdk_sessions() {
        let dir = tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("settings.json"),
            r#"{"permissions":{"deny":["bash(rm *)"]}}"#,
        )
        .unwrap();
        let policy = sdk_permission_policy(&agent_dir, dir.path());
        assert_eq!(policy.mode, davinci_agent::PermissionMode::AlwaysApprove);
        assert!(matches!(
            policy.decide(
                "c1",
                "bash",
                &serde_json::json!({"command":"rm -rf x"}),
                dir.path()
            ),
            davinci_agent::PermissionVerdict::Deny { .. }
        ));
    }

    #[test]
    fn sdk_queued_steer_and_follow_up_prepare_each_user_turn_before_provider() {
        fn assistant(id: &str) -> davinci_ai::AssistantMessage {
            davinci_ai::AssistantMessage {
                id: id.into(),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            }
        }

        let dir = tempdir().unwrap();
        let mut steer_session = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent-steer")),
            session_dir: Some(dir.path().join("sessions-steer")),
            append_system_prompt: vec!["SDK APPEND SENTINEL".into()],
            ..CreateAgentSessionOptions::default()
        })
        .unwrap()
        .session;
        steer_session.agent.queues.steer_mode = davinci_agent::QueueMode::OneAtATime;
        steer_session.steer(
            "Redesign this dashboard so it feels premium and intentional.",
            Vec::new(),
        );
        steer_session.steer("What is 2 + 2?", Vec::new());
        let mut steer_provider_prompts = Vec::new();
        steer_session
            .run(|agent| {
                steer_provider_prompts.push(agent.system_prompt.clone());
                Ok(assistant("sdk-steer"))
            })
            .unwrap();
        assert_eq!(steer_provider_prompts.len(), 2);
        let active = &steer_provider_prompts[0];
        assert!(active.contains("frontend_design_policy"));
        assert!(
            active.find("frontend_design_policy").unwrap()
                < active.find("SDK APPEND SENTINEL").unwrap(),
            "dynamic capability suffix must precede user append"
        );
        assert!(
            !steer_provider_prompts[1].contains("frontend_design_policy"),
            "later non-capability queued steer must deactivate frontend capability"
        );

        let mut follow_session = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent-follow")),
            session_dir: Some(dir.path().join("sessions-follow")),
            append_system_prompt: vec!["SDK FOLLOW APPEND".into()],
            ..CreateAgentSessionOptions::default()
        })
        .unwrap()
        .session;
        follow_session.agent.queues.follow_up_mode = davinci_agent::QueueMode::OneAtATime;
        follow_session.prompt("Start with a plain factual answer.");
        follow_session.follow_up(
            "Redesign this dashboard so it feels premium and intentional.",
            Vec::new(),
        );
        follow_session.follow_up("What is 2 + 2?", Vec::new());
        let mut follow_provider_prompts = Vec::new();
        follow_session
            .run(|agent| {
                follow_provider_prompts.push(agent.system_prompt.clone());
                Ok(assistant("sdk-follow"))
            })
            .unwrap();
        assert_eq!(follow_provider_prompts.len(), 3);
        assert!(!follow_provider_prompts[0].contains("frontend_design_policy"));
        let active = &follow_provider_prompts[1];
        assert!(active.contains("frontend_design_policy"));
        assert!(
            active.find("frontend_design_policy").unwrap()
                < active.find("SDK FOLLOW APPEND").unwrap(),
            "dynamic capability suffix must precede user append"
        );
        assert!(
            !follow_provider_prompts[2].contains("frontend_design_policy"),
            "later non-capability queued follow-up must deactivate frontend capability"
        );
    }

    #[test]
    fn sdk_default_all_queued_turns_union_capabilities_for_one_provider_request() {
        fn assistant(id: &str) -> davinci_ai::AssistantMessage {
            davinci_ai::AssistantMessage {
                id: id.into(),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            }
        }
        let dir = tempdir().unwrap();
        let mut session = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent-all")),
            session_dir: Some(dir.path().join("sessions-all")),
            append_system_prompt: vec!["SDK ALL APPEND".into()],
            ..CreateAgentSessionOptions::default()
        })
        .unwrap()
        .session;
        assert_eq!(
            session.agent.queues.steer_mode,
            davinci_agent::QueueMode::All
        );
        session.steer(
            "Redesign this dashboard so it feels premium and intentional.",
            Vec::new(),
        );
        session.steer("What is 2 + 2?", Vec::new());
        let mut prompts = Vec::new();
        session
            .run(|agent| {
                prompts.push(agent.system_prompt.clone());
                Ok(assistant("sdk-all"))
            })
            .unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("frontend_design_policy"));
        assert!(
            prompts[0].find("frontend_design_policy").unwrap()
                < prompts[0].find("SDK ALL APPEND").unwrap()
        );
    }

    #[test]
    fn sdk_session_prompt_activates_capabilities_on_user_turn() {
        let dir = tempdir().unwrap();
        let mut session = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            ..CreateAgentSessionOptions::default()
        })
        .unwrap()
        .session;

        assert!(!session
            .agent
            .system_prompt
            .contains("frontend_design_policy"));

        session.prompt("Redesign this dashboard so it feels premium and intentional.");

        assert!(
            session
                .agent
                .system_prompt
                .contains("frontend_design_policy"),
            "SDK user prompt must activate capability"
        );
        assert!(session
            .agent
            .prompt_manifest
            .as_ref()
            .unwrap()
            .modules
            .iter()
            .any(|m| m.id == "capability.frontend-design"));
    }

    #[test]
    fn custom_system_prompt_bypasses_astra_model_policy() {
        let dir = tempdir().unwrap();
        let result = create_agent_session(CreateAgentSessionOptions {
            cwd: Some(dir.path().to_path_buf()),
            agent_dir: Some(dir.path().join("agent")),
            session_dir: Some(dir.path().join("sessions")),
            provider: Some("openai-codex".into()),
            model: Some("gpt-6-astra".into()),
            system_prompt: Some("CUSTOM_ONLY_SENTINEL".into()),
            ..CreateAgentSessionOptions::default()
        })
        .expect("session");
        assert_eq!(result.session.agent.system_prompt, "CUSTOM_ONLY_SENTINEL");
        assert!(result.session.agent.prompt_session.is_custom());
        assert_eq!(
            result
                .session
                .agent
                .prompt_manifest
                .as_ref()
                .unwrap()
                .model_policy,
            "default"
        );
        assert!(!result.session.agent.system_prompt.contains("model.astra"));
    }
}

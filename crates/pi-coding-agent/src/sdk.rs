//! Library embed API matching `vendor/pi/packages/coding-agent/src/core/sdk.ts`.

use std::path::PathBuf;

use pi_agent::{default_system_prompt, load_context_files, Agent, BUILTIN_TOOLS};
use pi_ai::{find_model, load_builtin_models};
use pi_session::{default_agent_dir, JsonlSession};

const DEFAULT_ACTIVE_TOOLS: &[&str] = &["read", "bash", "edit", "write"];

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
}

pub struct AgentSession {
    pub agent: Agent,
    pub cwd: PathBuf,
    pub agent_dir: PathBuf,
}

impl AgentSession {
    pub fn prompt(&mut self, text: &str) -> pi_ai::ChatMessage {
        self.agent.prompt(text)
    }
}

pub struct CreateAgentSessionResult {
    pub session: AgentSession,
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
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = cwd.clone();
    agent.context_files = load_context_files(&cwd, true);

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

    if let Some(level) = options.thinking_level.as_deref() {
        agent.thinking_level = parse_thinking(level);
    }

    agent.tools = initial_tools(&options);
    agent.tool_registry = BUILTIN_TOOLS
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    let session_dir = options
        .session_dir
        .clone()
        .unwrap_or_else(pi_session::default_session_dir);
    agent.session = Some(
        JsonlSession::create(
            &session_dir,
            &cwd.to_string_lossy(),
            options.session_name.as_deref(),
        )
        .map_err(|err| err.to_string())?,
    );

    Ok(CreateAgentSessionResult {
        session: AgentSession {
            agent,
            cwd,
            agent_dir,
        },
        model_fallback_message,
    })
}

fn initial_tools(options: &CreateAgentSessionOptions) -> Vec<String> {
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
}

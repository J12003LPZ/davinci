//! Session-level sizing for dynamic workflows.
//!
//! No TypeScript counterpart. Mirrors Claude Code's workflow controls
//! (https://code.claude.com/docs/en/workflows): the `workflowSizeGuideline`
//! setting tells the model how many agents to aim for, and
//! `CLAUDE_CODE_WORKFLOW_MAX_CONCURRENT_AGENTS` bounds how many run at once.
//! Here the same knobs are the `workflowSizeGuideline` and
//! `workflowMaxConcurrentAgents` settings (env
//! `DAVINCI_WORKFLOW_MAX_CONCURRENT_AGENTS` overrides the second).

/// Largest `max_parallel_agents` a spec may declare, and the ceiling of the
/// concurrency setting (Claude Code accepts 1 to 256).
pub const MAX_CONCURRENT_AGENTS_CAP: usize = 256;
/// Agents one run may schedule in total (Claude Code: 1,000 per run).
pub const MAX_TOTAL_AGENTS_CAP: usize = 1_000;
/// Default concurrency ceiling (Claude Code's default is 16).
pub const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 16;
/// Agent count past which a run is flagged as a large workflow when no size
/// guideline was chosen explicitly (Claude Code uses 25).
pub const LARGE_WORKFLOW_AGENTS: usize = 25;

/// How many agents the model should aim for when it writes a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowSizeGuideline {
    /// No guideline: size the workflow to the task.
    Unrestricted,
    /// Fewer than 5 agents.
    Small,
    /// Fewer than 10 agents.
    #[default]
    Medium,
    /// Fewer than 50 agents.
    Large,
}

impl WorkflowSizeGuideline {
    pub const ALL: [Self; 4] = [Self::Unrestricted, Self::Small, Self::Medium, Self::Large];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "unrestricted" => Some(Self::Unrestricted),
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            "large" => Some(Self::Large),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unrestricted => "unrestricted",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// The agent count the model is asked to stay under, if any.
    pub fn agent_target(self) -> Option<usize> {
        match self {
            Self::Unrestricted => None,
            Self::Small => Some(5),
            Self::Medium => Some(10),
            Self::Large => Some(50),
        }
    }
}

/// The workflow knobs a session runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowSettings {
    pub size_guideline: WorkflowSizeGuideline,
    /// The guideline came from the user rather than the default. Only then
    /// does its agent count replace the large-workflow threshold.
    pub size_guideline_explicit: bool,
    pub max_concurrent_agents: usize,
}

impl Default for WorkflowSettings {
    fn default() -> Self {
        Self {
            size_guideline: WorkflowSizeGuideline::default(),
            size_guideline_explicit: false,
            max_concurrent_agents: DEFAULT_MAX_CONCURRENT_AGENTS,
        }
    }
}

impl WorkflowSettings {
    /// Build from raw setting values. `env_concurrency` is the
    /// `DAVINCI_WORKFLOW_MAX_CONCURRENT_AGENTS` value and wins over the
    /// setting; out-of-range or unparsable values fall back rather than fail.
    pub fn resolve(
        size_guideline: Option<&str>,
        max_concurrent_agents: Option<u64>,
        env_concurrency: Option<&str>,
    ) -> Self {
        let parsed_guideline = size_guideline.and_then(WorkflowSizeGuideline::parse);
        let from_env = env_concurrency
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|value| (1..=MAX_CONCURRENT_AGENTS_CAP as u64).contains(value));
        let from_setting = max_concurrent_agents
            .filter(|value| (1..=MAX_CONCURRENT_AGENTS_CAP as u64).contains(value));
        Self {
            size_guideline: parsed_guideline.unwrap_or_default(),
            size_guideline_explicit: parsed_guideline.is_some(),
            max_concurrent_agents: from_env
                .or(from_setting)
                .map(|value| value as usize)
                .unwrap_or(DEFAULT_MAX_CONCURRENT_AGENTS),
        }
    }

    /// Agents a run may schedule before it is flagged as large.
    pub fn large_workflow_threshold(&self) -> usize {
        if self.size_guideline_explicit {
            self.size_guideline
                .agent_target()
                .unwrap_or(LARGE_WORKFLOW_AGENTS)
        } else {
            LARGE_WORKFLOW_AGENTS
        }
    }

    /// Sentence appended to the `workflow_run` tool description.
    pub fn tool_guidance(&self) -> String {
        let size = match self.size_guideline.agent_target() {
            Some(target) => format!(
                "Size guideline ({}): aim for fewer than {target} agents in total unless the user asks for more.",
                self.size_guideline.as_str()
            ),
            None => "Size guideline (unrestricted): size the workflow to the task.".to_string(),
        };
        format!(
            "{size} At most {} workers run at once; set max_parallel_agents at or below that.",
            self.max_concurrent_agents
        )
    }

    /// Effective concurrency for one run: the spec's request, bounded by the
    /// session ceiling.
    pub fn concurrency_for(&self, requested: usize) -> usize {
        requested.clamp(
            1,
            self.max_concurrent_agents
                .clamp(1, MAX_CONCURRENT_AGENTS_CAP),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_claude_code() {
        let settings = WorkflowSettings::resolve(None, None, None);
        assert_eq!(settings.size_guideline, WorkflowSizeGuideline::Medium);
        assert!(!settings.size_guideline_explicit);
        assert_eq!(settings.max_concurrent_agents, 16);
        assert_eq!(settings.large_workflow_threshold(), 25);
    }

    #[test]
    fn env_overrides_setting_and_bad_values_fall_back() {
        assert_eq!(
            WorkflowSettings::resolve(None, Some(4), Some("32")).max_concurrent_agents,
            32
        );
        assert_eq!(
            WorkflowSettings::resolve(None, Some(4), Some("0")).max_concurrent_agents,
            4
        );
        assert_eq!(
            WorkflowSettings::resolve(None, Some(999), Some("junk")).max_concurrent_agents,
            16
        );
        assert_eq!(
            WorkflowSettings::resolve(None, Some(256), None).max_concurrent_agents,
            256
        );
    }

    #[test]
    fn an_explicit_guideline_moves_the_large_threshold() {
        let small = WorkflowSettings::resolve(Some("small"), None, None);
        assert!(small.size_guideline_explicit);
        assert_eq!(small.large_workflow_threshold(), 5);
        let unrestricted = WorkflowSettings::resolve(Some("Unrestricted"), None, None);
        assert_eq!(unrestricted.large_workflow_threshold(), 25);
        let bogus = WorkflowSettings::resolve(Some("huge"), None, None);
        assert_eq!(bogus.size_guideline, WorkflowSizeGuideline::Medium);
        assert!(!bogus.size_guideline_explicit);
    }

    #[test]
    fn concurrency_is_bounded_by_the_session_ceiling() {
        let settings = WorkflowSettings::resolve(None, Some(3), None);
        assert_eq!(settings.concurrency_for(8), 3);
        assert_eq!(settings.concurrency_for(2), 2);
        assert_eq!(settings.concurrency_for(0), 1);
    }

    #[test]
    fn tool_guidance_names_the_target_and_ceiling() {
        let text = WorkflowSettings::resolve(Some("large"), Some(12), None).tool_guidance();
        assert!(text.contains("fewer than 50 agents"));
        assert!(text.contains("At most 12 workers"));
        let text = WorkflowSettings::resolve(Some("unrestricted"), None, None).tool_guidance();
        assert!(text.contains("size the workflow to the task"));
    }
}

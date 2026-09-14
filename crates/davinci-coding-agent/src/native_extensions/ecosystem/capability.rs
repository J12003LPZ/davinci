//! Deterministic capability assembly for one graph worker.

use crate::native_extensions::ecosystem::{
    build_context_packet, ContextPacket, ContextPacketRequest, DEFAULT_GRAPH_CONTEXT_TOKENS,
};
use crate::native_extensions::graph::Role;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest<'a> {
    pub prompt: &'a str,
    pub role: Role,
    pub context_token_cap: usize,
    pub include_skills: bool,
}

impl<'a> CapabilityRequest<'a> {
    pub fn new(prompt: &'a str, role: Role) -> Self {
        Self {
            prompt,
            role,
            context_token_cap: DEFAULT_GRAPH_CONTEXT_TOKENS,
            include_skills: true,
        }
    }

    pub fn with_context_token_cap(mut self, cap: usize) -> Self {
        self.context_token_cap = cap;
        self
    }

    pub fn with_skills(mut self, include: bool) -> Self {
        self.include_skills = include;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySelection {
    pub tools: Vec<String>,
    pub context: ContextPacket,
}

pub fn select_capabilities(
    memory: &crate::native_extensions::VectorMemory,
    learning: &crate::native_extensions::LearningController,
    authorized_tools: Vec<String>,
    request: CapabilityRequest<'_>,
) -> CapabilitySelection {
    let context_request = ContextPacketRequest::new(request.prompt)
        .with_role(request.role)
        .with_token_cap(request.context_token_cap)
        .with_skills(request.include_skills);

    CapabilitySelection {
        tools: authorized_tools,
        context: build_context_packet(memory, learning, context_request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_selection_preserves_tools_and_context_budget() {
        let dir = tempfile::tempdir().unwrap();
        let memory = crate::native_extensions::VectorMemory::new(dir.path().to_path_buf());
        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);
        let tools = crate::native_extensions::graph::roles::role_tools(
            crate::native_extensions::graph::Role::Researcher,
        );

        let selection = select_capabilities(
            &memory,
            &learning,
            tools.clone(),
            CapabilityRequest::new(
                "research the parser failure",
                crate::native_extensions::graph::Role::Researcher,
            )
            .with_context_token_cap(500),
        );

        assert_eq!(selection.tools, tools);
        assert!(selection.context.estimated_tokens <= 500);
    }
}

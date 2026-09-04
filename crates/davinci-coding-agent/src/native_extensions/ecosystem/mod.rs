//! Bounded ecosystem integration contracts for Graph, Token Governor, Vector Memory, and Learning.

pub mod cache_affinity;
pub mod context;
pub mod resource;

#[allow(unused_imports)]
pub use cache_affinity::*;
#[allow(unused_imports)]
pub use context::*;
#[allow(unused_imports)]
pub use resource::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_graph_context_caps_match_design() {
        assert_eq!(DEFAULT_GRAPH_CONTEXT_TOKENS, 2_500);
        assert_eq!(DEFAULT_GRAPH_MEMORY_TOKENS, 1_200);
        assert_eq!(DEFAULT_GRAPH_MEMORY_HITS, 4);
        assert_eq!(DEFAULT_GRAPH_SKILL_TOKENS, 1_000);
        assert_eq!(DEFAULT_GRAPH_SKILL_COUNT, 2);
    }
}

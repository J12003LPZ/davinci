//! Collaboration prompt module: user intent, communication, question discipline.

use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub fn collaboration_user_intent_module() -> PromptModule {
    PromptModule {
        id: "collaboration.user-intent".to_string(),
        version: 2,
        cache_class: PromptCacheClass::Stable,
        body: "\
Focus on the user's explicit intent. When the user provides instructions, prioritize them \
over generic habits. Preserve unrelated user work. Ask focused questions only when critical \
requirements or design choices are ambiguous and cannot be deduced from the repository. \
When several such decisions are open at once, ask them together in one structured question \
call rather than one per turn, and wait for the answers before acting on them."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collaboration_prioritizes_user_intent() {
        let text = collaboration_user_intent_module().body;
        assert!(text.contains("explicit intent"));
        assert!(text.contains("Preserve unrelated user work"));
        assert!(text.contains("ask them together in one structured question"));
    }
}

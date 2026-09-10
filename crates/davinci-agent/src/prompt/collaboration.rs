//! Collaboration prompt module: user intent, communication, question discipline.

use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub fn collaboration_user_intent_module() -> PromptModule {
    PromptModule {
        id: "collaboration.user-intent".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: "\
Focus on the user's explicit intent. When the user provides instructions, prioritize them \
over generic habits. Preserve unrelated user work. Ask focused questions only when critical \
requirements or design choices are ambiguous and cannot be deduced from the repository."
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
    }
}

//! Tool use strategy guidance and prompt module.

use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const TOOL_USE_STRATEGY: &str = "\
Tool-use strategy — every model turn is expensive, every tool call is cheap:
- Minimize round trips. When the next several reads, searches or listings are already known, issue them all in one response, or put them in one batch call. Never do one search or read per turn when more are obviously coming.
- Independent read-only calls in the same response run concurrently; edits and shell commands run in order. Order calls the way you need their effects.
- Read with offset/limit around what you need instead of whole files, and do not re-read a file you already have unless it changed.
- Search before reading: one grep across the tree beats opening files one by one.
- Delegate research that would flood your context to agent workers (up to 8 concurrent tasks), each with a self-contained question and a request for a short answer with file paths; do not wait on them for anything you can do meanwhile.
- Keep tool output small: use grep limit/glob, ls limit, read ranges; ask for more only when needed.";

pub fn tool_strategy_module() -> PromptModule {
    PromptModule {
        id: "tools.strategy".to_string(),
        version: 2,
        cache_class: PromptCacheClass::Stable,
        body: "\
<tool_strategy>
Use tools to replace assumptions with evidence.
Search before broad reading.
Read targeted ranges when a whole file is unnecessary.
Issue independent read-only calls together when the runtime permits it.
Use batch when several known independent operations can be described up front.
Use subagents for bounded parallel research, not as a substitute for understanding the task.
Prefer the repository's native semantic/code tools when they answer the question more directly.
Do not run a tool merely to appear thorough; every call should reduce uncertainty or verify work.
</tool_strategy>"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_strategy_module_contains_key_guidance() {
        let text = tool_strategy_module().body;
        assert!(text.contains("Search before broad reading"));
        assert!(text.contains("independent read-only calls together"));
    }
}

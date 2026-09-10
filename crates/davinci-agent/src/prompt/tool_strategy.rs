//! Tool use strategy guidance.

pub const TOOL_USE_STRATEGY: &str = "\
Tool-use strategy — every model turn is expensive, every tool call is cheap:
- Minimize round trips. When the next several reads, searches or listings are already known, issue them all in one response, or put them in one batch call. Never do one search or read per turn when more are obviously coming.
- Independent read-only calls in the same response run concurrently; edits and shell commands run in order. Order calls the way you need their effects.
- Read with offset/limit around what you need instead of whole files, and do not re-read a file you already have unless it changed.
- Search before reading: one grep across the tree beats opening files one by one.
- Delegate research that would flood your context to agent workers (up to 8 concurrent tasks), each with a self-contained question and a request for a short answer with file paths; do not wait on them for anything you can do meanwhile.
- Keep tool output small: use grep limit/glob, ls limit, read ranges; ask for more only when needed.";

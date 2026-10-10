//! Default-off sandbox execution contracts and Rust-owned capability policy.
mod authority;
mod deadline;
mod dispatch;
mod execution;
mod policy;
pub mod projection;
mod types;
pub(crate) use authority::fingerprint as authority_fingerprint;
pub(crate) use deadline::remaining_root_wall_ms;
pub use dispatch::AgentCodeModeBroker;
pub(crate) use execution::mandatory_facts;
pub(crate) use execution::CodeModeBinding;
pub use policy::{js_name, CapabilityPolicy};
pub use types::*;

impl Default for CodeModeMode {
    fn default() -> Self {
        Self::Off
    }
}

impl Default for CodeModeLimits {
    fn default() -> Self {
        Self {
            script_bytes: 65536,
            wall_ms: 60000,
            vm_heap_bytes: 67108864,
            tool_calls: 64,
            parallelism: 4,
            pending_calls: 64,
            argument_bytes: 65536,
            json_depth: 64,
            child_result_bytes: 1048576,
            total_result_bytes: 8388608,
            frame_bytes: 2097152,
            output_bytes: 16384,
            collected_output_bytes: 1048576,
            metadata_calls: 64,
            metadata_bytes: 524288,
            cleanup_grace_ms: 2000,
        }
    }
}

/// Whether the script opens with an option-header directive. Option
/// headers are not a supported control channel, so they are refused rather
/// than silently ignored. Only the header counts: the leading run of blank
/// and `//` lines. The first line of code ends it, so the same text later in
/// the script (a comment, or a line inside a template literal) is content.
fn has_option_header(code: &str) -> bool {
    for line in code.lines() {
        let line = line.trim_start();
        if line.starts_with("// @options:") || line.starts_with("// @codemode") {
            return true;
        }
        if !line.is_empty() && !line.starts_with("//") {
            return false;
        }
    }
    false
}

impl CodeModeLimits {
    pub fn for_request(&self, request: &CodeModeRequest) -> Result<Self, CodeModeError> {
        let mut limits = self.clone();
        if request.code.trim().is_empty() || has_option_header(&request.code) {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "nonempty source without option-header directives is required",
            ));
        }
        if request.code.len() > self.script_bytes.min(65536) {
            return Err(CodeModeError::new(
                "LIMIT_EXCEEDED",
                "script exceeds byte limit",
            ));
        }
        if let Some(timeout) = request.timeout_ms {
            if timeout == 0 || timeout > 300000 {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "timeout exceeds admitted range",
                ));
            }
            limits.wall_ms = timeout.min(self.wall_ms);
        }
        if let Some(output) = request.max_output_bytes {
            if !(4096..=65536).contains(&output) {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "output limit exceeds admitted range",
                ));
            }
            limits.output_bytes = output.min(self.output_bytes);
        }
        Ok(limits)
    }
}

impl CodeModeError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.chars().take(1024).collect(),
            operation_ref: None,
        }
    }
}

//! Default-off sandbox execution contracts and Rust-owned capability policy.
mod authority;
mod deadline;
mod dispatch;
mod execution;
mod policy;
pub mod projection;
mod recovery;
mod runs;
mod script_log;
mod types;
pub(crate) use authority::fingerprint as authority_fingerprint;
pub(crate) use deadline::remaining_root_wall_ms;
pub use dispatch::AgentCodeModeBroker;
pub(crate) use execution::mandatory_facts;
pub(crate) use execution::CodeModeBinding;
pub use policy::{effect_is_admitted, js_name, resolve_script_alias, CapabilityPolicy};
pub use recovery::{reconcile_script_child_effect, resolve_script_effect, ScriptEffectResolution};
pub use runs::{
    reconcile_script_run, ReconciledScriptRun, ScriptRunError, ScriptRunRecord, ScriptRunRegistry,
    ScriptRunState,
};
pub use script_log::{
    ScriptLogError, ScriptLogEvent, ScriptLogRef, ScriptLogSlice, ScriptLogStore,
};
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
            output_events: 1024,
            input_bytes: 1048576,
            collected_output_bytes: 1048576,
            metadata_calls: 64,
            metadata_bytes: 524288,
            cleanup_grace_ms: 2000,
        }
    }
}

/// Bounded mirror of the host's raw-source lexer. The host owns the
/// authoritative parse; this refuses a header that cannot be well formed
/// before any child is dispatched.
pub fn validate_script_source(code: &str, input_limit: usize) -> Result<(), CodeModeError> {
    fn check_name(name: &str) -> bool {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    fn check_tag(tag: &str) -> bool {
        !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    let mut inputs: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut directive = false;
    let mut input_bytes = 0usize;
    let lines: Vec<&str> = code.split('\n').collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(spec) = line.strip_prefix("// @input: ") {
            let (name, rest) = spec.split_once(" <<'").ok_or_else(|| {
                CodeModeError::new("INVALID_INPUT", "malformed script input marker")
            })?;
            let tag = rest
                .strip_suffix('\'')
                .filter(|tag| check_tag(tag) && rest.matches('\'').count() == 1)
                .ok_or_else(|| {
                    CodeModeError::new("INVALID_INPUT", "malformed script input marker")
                })?;
            if !check_name(name) {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "malformed script input name",
                ));
            }
            if matches!(name, "__proto__" | "constructor" | "prototype") {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "reserved script input name",
                ));
            }
            if !inputs.insert(name) {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "duplicate script input",
                ));
            }
            let start = index + 1;
            let end = lines[start..]
                .iter()
                .position(|candidate| *candidate == tag)
                .map(|offset| start + offset)
                .ok_or_else(|| CodeModeError::new("INVALID_INPUT", "unclosed script input"))?;
            input_bytes += lines[start..end]
                .iter()
                .map(|line| line.len() + 1)
                .sum::<usize>();
            if input_bytes > input_limit {
                return Err(CodeModeError::new(
                    "LIMIT_EXCEEDED",
                    "script input aggregate exceeds limit",
                ));
            }
            index = end + 1;
            continue;
        }
        if line.starts_with("// @input") {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "malformed script input marker",
            ));
        }
        if let Some(raw) = line.strip_prefix("// @script: ") {
            if directive {
                return Err(CodeModeError::new(
                    "INVALID_INPUT",
                    "duplicate script directive",
                ));
            }
            directive = true;
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct ScriptDirective {
                #[serde(default)]
                start_in_background: Option<bool>,
            }
            let parsed = serde_json::from_str::<ScriptDirective>(raw)
                .map_err(|_| CodeModeError::new("INVALID_INPUT", "malformed script directive"))?;
            // Parsed and bounded here; the host decides when to detach.
            let _requested_background = parsed.start_in_background;
            index += 1;
            continue;
        }
        if line.starts_with("// @script") {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "malformed script directive",
            ));
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            index += 1;
            continue;
        }
        break;
    }
    Ok(())
}

/// Whether the script opens with an option-header directive. Option
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

/// True when the raw source asks the host to detach. The directive is parsed
/// by the same bounded lexer that validates the rest of the header.
pub fn script_requests_background(code: &str) -> bool {
    for line in code.lines() {
        let line = line.trim_start();
        if let Some(raw) = line.strip_prefix("// @script: ") {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Directive {
                #[serde(default)]
                start_in_background: Option<bool>,
            }
            return serde_json::from_str::<Directive>(raw)
                .ok()
                .and_then(|directive| directive.start_in_background)
                .unwrap_or(false);
        }
        if !line.is_empty() && !line.starts_with("//") {
            break;
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
        validate_script_source(&request.code, limits.input_bytes)?;
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

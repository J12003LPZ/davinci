use super::{admission::*, error::*, host::DesignHostLease, runtime::*, types::*};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledDesign {
    pub source_hash: String,
    pub files: BTreeMap<String, String>,
    pub compiler_version: String,
}
pub fn compile(
    ctx: &AuthorizedDesignContext,
    runtime: &TrustedDesignRuntime,
    entry: &str,
    files: &BTreeMap<String, String>,
) -> DesignResult<CompiledDesign> {
    ctx.check(
        "design_render",
        &json!({"entry":entry,"runtime":runtime.fingerprint()}),
    )?;
    validate_path(entry)?;
    if files.is_empty() || files.len() > 64 || !files.contains_key(entry) {
        return Err(DesignError::InvalidInput(
            "invalid compile entry or file count".into(),
        ));
    }
    let mut total = 0u64;
    for (name, text) in files {
        validate_path(name)?;
        total = total.saturating_add(text.len() as u64);
        if text.len() > 256 * 1024 || total > 2 * 1024 * 1024 {
            return Err(DesignError::BudgetExceeded("compile source limit".into()));
        }
    }
    runtime.verify()?;
    let mut host = DesignHostLease::spawn(
        ctx,
        runtime,
        "host/compile.cjs",
        json!({"entry":entry,"files":files}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        ctx.check(
            "design_render",
            &json!({"entry":entry,"runtime":runtime.fingerprint()}),
        )?;
        if Instant::now() >= deadline {
            return Err(DesignError::BudgetExceeded(
                "compile deadline exceeded".into(),
            ));
        }
        if let Some(message) = host.receive(Duration::from_millis(50))? {
            if message["id"] != "ready" || message.get("error").is_some() {
                return Err(DesignError::InvalidInput(
                    "virtual source compilation failed".into(),
                ));
            }
            let path = host.compiled_output();
            no_links(&path)?;
            let size = fs::metadata(&path)?.len();
            if size > 12 * 1024 * 1024
                || message["result"]["size"].as_u64() != Some(size)
                || message["result"]["sha256"].as_str() != Some(file_hash(&path)?.as_str())
            {
                return Err(DesignError::CorruptArtifact(
                    "compiled transfer mismatch".into(),
                ));
            }
            let compiled: CompiledDesign = serde_json::from_slice(&fs::read(path)?)?;
            if compiled.source_hash != super::store::digest(files)?
                || compiled.compiler_version != "0.25.11"
                || compiled.files.len() > 64
                || !compiled.files.contains_key(if entry.ends_with(".html") {
                    entry
                } else {
                    "index.html"
                })
            {
                return Err(DesignError::CorruptArtifact(
                    "invalid compiled bundle".into(),
                ));
            }
            let mut total = 0u64;
            for (name, text) in &compiled.files {
                validate_path(name)?;
                total = total.saturating_add(text.len() as u64);
            }
            if total > 8 * 1024 * 1024 {
                return Err(DesignError::BudgetExceeded("compiled bundle limit".into()));
            }
            return Ok(compiled);
        }
    }
}

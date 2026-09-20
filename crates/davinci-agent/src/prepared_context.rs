use crate::{runtime::ContextImage, Agent};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Write, sync::Arc};

/// One immutable projection, including failures, per complete input revision.
#[derive(Debug, Clone)]
pub struct PreparedContextImage {
    pub revision: String,
    pub image: Result<Arc<ContextImage>, String>,
}

struct Fingerprint(Sha256);
impl Write for Fingerprint {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Agent {
    fn context_image_revision(&self) -> String {
        let mut out = Fingerprint(Sha256::new());
        // Public mutable fields remain compatible: stream their fingerprint
        // without allocating/cloning the transcript or traversing its branch.
        out.feed(&(&self.messages, &self.ephemeral_context, &self.context_files));
        if let Some(session) = &self.session {
            out.feed(&(
                &session.path,
                &session.header.id,
                &session.leaf_id,
                &session.entries,
            ));
        }
        out.feed(&(
            &self.system_prompt,
            &self.provider_system_prompt_suffix,
            &self.provider,
            &self.model_id,
            self.context_window,
            self.thinking_level,
            &self.thinking_budgets,
            self.provider_output_limit,
            self.provider_context_overhead_tokens,
            self.compaction.reserve_tokens,
            self.block_images,
            self.prepared_context_generation,
        ));
        out.feed(&self.provider_tool_schema_identity());
        out.feed(&self.visible_tool_names());
        out.feed(&self.plan_provider_context());
        if let Some(runtime) = &self.runtime {
            out.feed(&(
                runtime.run_id,
                runtime.agent_id,
                runtime.context_broker.revision(),
                runtime.context_vm.root(),
            ));
        }
        format!("{:x}", out.0.finalize())
    }

    pub fn prepared_context_image(&self) -> Result<Arc<ContextImage>, String> {
        let mut cache = self
            .prepared_context_image
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let revision = self.context_image_revision();
        if let Some(prepared) = cache.as_ref().filter(|p| p.revision == revision) {
            return prepared.image.clone();
        }
        let image = self.build_context_vm_image().map(Arc::new);
        *cache = Some(PreparedContextImage {
            revision: self.context_image_revision(),
            image: image.clone(),
        });
        image
    }

    pub fn context_vm_image(&self) -> Result<ContextImage, String> {
        self.prepared_context_image().map(|image| (*image).clone())
    }

    /// Host calls this when external context changes without changing its data
    /// fields. The run loop also invalidates at each new model/tool revision.
    pub fn invalidate_context_image(&mut self) {
        self.prepared_context_generation = self.prepared_context_generation.wrapping_add(1);
    }
}

impl Fingerprint {
    fn feed<T: Serialize>(&mut self, value: &T) {
        serde_json::to_writer(&mut *self, value).expect("context fields serialize");
        self.0.update(b"\n");
    }
}

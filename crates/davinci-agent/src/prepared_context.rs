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

struct Fingerprint(Sha256, usize);
impl Write for Fingerprint {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        self.1 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Transcript bytes streamed through `history_fingerprint` on this thread.
    static HISTORY_BYTES_HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Agent {
    /// The O(history) part of the revision. Building an image only takes
    /// `&self`, so the transcript cannot change during a build and one digest
    /// serves both the lookup and the post-build revision.
    fn history_fingerprint(&self) -> String {
        let mut out = Fingerprint(Sha256::new(), 0);
        // Public mutable fields remain compatible: stream their fingerprint
        // without allocating/cloning the transcript or traversing its branch.
        out.feed(&(
            &self.messages,
            &self.ephemeral_context,
            &self.context_files,
            &self.completion_context,
        ));
        if let Some(session) = &self.session {
            out.feed(&(
                &session.path,
                &session.header.id,
                &session.leaf_id,
                &session.entries,
            ));
        }
        #[cfg(test)]
        HISTORY_BYTES_HASHED.with(|hashed| hashed.set(hashed.get() + out.1));
        format!("{:x}", davinci_sys::hex::Lower(&out.0.finalize()))
    }

    fn context_image_revision(&self, history: &str) -> String {
        let mut out = Fingerprint(Sha256::new(), 0);
        out.feed(&history);
        out.feed(&(
            &self.system_prompt,
            &self.provider_system_prompt_suffix,
            &self.provider,
            &self.model_id,
            self.context_window,
            self.thinking_level,
            &self.thinking_budgets,
            self.provider_output_limit,
            self.provider_context_overhead_tokens(),
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
        format!("{:x}", davinci_sys::hex::Lower(&out.0.finalize()))
    }

    pub fn prepared_context_image(&self) -> Result<Arc<ContextImage>, String> {
        let _timing = self.counters.digest_retrieval.start();
        let mut cache = self
            .prepared_context_image
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let history = self.history_fingerprint();
        let revision = self.context_image_revision(&history);
        if let Some(prepared) = cache.as_ref().filter(|p| p.revision == revision) {
            return prepared.image.clone();
        }
        let initial_budget = self.provider_context_budget();
        let mut image = self.build_context_vm_image();
        // Paging may expose retrieve_context during compilation. Recompile once
        // with that schema reserved, rather than caching the old admission under
        // the new schema revision. Any further change must still fit in full.
        if image.is_ok() && self.provider_context_budget() != initial_budget {
            image = self.build_context_vm_image();
        }
        let final_budget = self.provider_context_budget();
        if image.as_ref().is_ok_and(|image| {
            final_budget.reserved() >= final_budget.window
                || image.estimated_tokens > final_budget.working_set_budget()
        }) {
            image = Err(crate::runtime::context_vm::CONTEXT_BUDGET_EXCEEDED.into());
        }
        let image = image.map(Arc::new);
        *cache = Some(PreparedContextImage {
            revision: self.context_image_revision(&history),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::context_vm::ContextVmMode;
    use crate::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    use davinci_ai::ChatMessage;

    fn hashed() -> usize {
        HISTORY_BYTES_HASHED.with(std::cell::Cell::get)
    }

    #[test]
    fn wor64_cache_miss_hashes_the_transcript_once() {
        let mut agent = Agent::new("system");
        agent.set_runtime(RuntimeHandle::new(
            RunId::new(),
            AgentId::new(),
            RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(ContextVmMode::Active);
        for index in 0..200 {
            agent.messages.push(ChatMessage::text(
                "user",
                &format!("turn {index} {}", "x".repeat(200)),
            ));
        }
        let one_scan = {
            let before = hashed();
            agent.history_fingerprint();
            hashed() - before
        };
        assert!(one_scan > 40_000);

        let before = hashed();
        agent.prepared_context_image().unwrap();
        assert_eq!(
            hashed() - before,
            one_scan,
            "miss scanned the history more than once"
        );

        let before = hashed();
        agent.prepared_context_image().unwrap();
        assert_eq!(
            hashed() - before,
            one_scan,
            "hit scanned the history more than once"
        );
    }
}

use davinci_agent::{runtime::ContextVmMode, Agent};
use davinci_ai::{content_text, ChatMessage};
use davinci_session::{JsonlSession, SessionEntry};
use std::{sync::Arc, time::Instant};

fn append(session: &mut JsonlSession, message: ChatMessage) {
    let seq = session.entries.len() as u64 + 1;
    session
        .append_entry(SessionEntry {
            id: format!("event-{seq}"),
            entry_type: "message".into(),
            parent_id: session.leaf_id.clone(),
            seq,
            timestamp: 0,
            message: Some(serde_json::to_value(message).unwrap()),
            custom_type: None,
            extra: Default::default(),
        })
        .unwrap();
}

fn quantiles(mut samples: Vec<u64>) -> serde_json::Value {
    samples.sort_unstable();
    let n = samples.len();
    serde_json::json!({"p50_us": samples[n / 2], "p95_us": samples[(n * 95).div_ceil(100) - 1],
        "p99_us": samples[(n * 99).div_ceil(100) - 1]})
}

#[test]
#[ignore = "paired context latency and logical memory measurement; run with --ignored --nocapture"]
fn context_image_cold_and_warm_performance() {
    const SAMPLES: usize = 16;
    for turns in [100, 300] {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            JsonlSession::create(directory.path(), "benchmark-fixture", None).unwrap();
        for turn in 0..turns {
            append(
                &mut session,
                ChatMessage::text("user", format!("Inspect component {turn}.")),
            );
            append(
                &mut session,
                ChatMessage::tool_result(
                    format!("call-{turn}"),
                    "read",
                    format!("component {turn} evidence: {}", "source body\n".repeat(700)),
                    false,
                ),
            );
            append(
                &mut session,
                ChatMessage::text("assistant", "Inspection recorded."),
            );
        }
        append(
            &mut session,
            ChatMessage::text("user", "Continue from the latest evidence."),
        );
        let mut agent = Agent::new("Keep source evidence traceable.");
        agent.load_from_session(session).unwrap();
        agent.set_context_vm_mode(ContextVmMode::Active);
        let history_bytes: usize = agent
            .messages
            .iter()
            .map(|m| content_text(&m.content).len())
            .sum();
        let history_wire_bytes =
            serde_json::to_vec(&davinci_ai::openai_responses_input(&agent.messages))
                .unwrap()
                .len();
        let history_token_ceiling: u64 = agent
            .messages
            .iter()
            .map(davinci_agent::provider_budget::message_token_ceiling)
            .sum();
        let mut cold = Vec::new();
        let mut warm = Vec::new();
        let mut last_image = None;
        for _ in 0..SAMPLES {
            agent.invalidate_context_image();
            let started = Instant::now();
            let image = agent.prepared_context_image().unwrap();
            cold.push(started.elapsed().as_micros() as u64);
            let compiled = agent
                .runtime
                .as_ref()
                .unwrap()
                .context_vm
                .metrics()
                .images_compiled;
            let started = Instant::now();
            let reused = agent.prepared_context_image().unwrap();
            warm.push(started.elapsed().as_micros() as u64);
            assert!(Arc::ptr_eq(&image, &reused));
            assert_eq!(
                compiled,
                agent
                    .runtime
                    .as_ref()
                    .unwrap()
                    .context_vm
                    .metrics()
                    .images_compiled
            );
            assert!(image.estimated_tokens <= agent.provider_context_budget().working_set_budget());
            last_image = Some(image);
        }
        let image = last_image.unwrap();
        let retained = agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .resident_source_bytes();
        assert_eq!(retained, 0);
        println!(
            "CONTEXT_VM_PERF {}",
            serde_json::json!({
                "turns": turns, "samples": SAMPLES, "first_cold_us": cold[0],
                "recompiled": quantiles(cold), "reused": quantiles(warm),
                "images_compiled": agent.runtime.as_ref().unwrap().context_vm.metrics().images_compiled,
                "history_body_bytes": history_bytes, "vm_retained_source_body_bytes": retained,
                "old_two_copy_body_bytes": history_bytes * 2,
                "history_provider_wire_bytes": history_wire_bytes,
                "image_provider_wire_bytes": serde_json::to_vec(&davinci_ai::openai_responses_input(&image.messages)).unwrap().len(),
                "history_token_ceiling": history_token_ceiling,
                "image_token_ceiling": image.estimated_tokens,
            })
        );
    }
}

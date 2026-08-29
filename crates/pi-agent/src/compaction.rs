use crate::events::AgentMessage;

#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub summary: String,
    pub retained_tail: Vec<AgentMessage>,
    pub tokens_before: usize,
}

pub fn compact_messages(
    messages: &[AgentMessage],
    custom_instructions: Option<&str>,
    keep_tail: usize,
) -> CompactionResult {
    let tokens_before = messages
        .iter()
        .map(|m| m.content.split_whitespace().count())
        .sum();
    let keep = keep_tail.min(messages.len());
    let (head, tail) = messages.split_at(messages.len().saturating_sub(keep));
    let mut summary = if head.is_empty() {
        "No earlier context.".to_string()
    } else {
        let mut bits = Vec::new();
        for message in head {
            let excerpt: String = message.content.chars().take(200).collect();
            bits.push(format!("{}: {excerpt}", message.role));
        }
        format!(
            "Compacted {} earlier messages.\n{}",
            head.len(),
            bits.join("\n")
        )
    };
    if let Some(extra) = custom_instructions {
        summary.push_str("\n\n");
        summary.push_str(extra);
    }
    CompactionResult {
        summary,
        retained_tail: tail.to_vec(),
        tokens_before,
    }
}

pub fn should_auto_compact(tokens: usize, context_window: usize) -> bool {
    context_window > 0 && tokens * 4 > context_window * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_tail_and_summarizes_head() {
        let messages: Vec<AgentMessage> = (0..5)
            .map(|i| AgentMessage {
                role: "user".into(),
                content: format!("msg {i}"),
                images: vec![],
            })
            .collect();
        let result = compact_messages(&messages, Some("focus on auth"), 2);
        assert_eq!(result.retained_tail.len(), 2);
        assert!(result.summary.contains("Compacted 3"));
        assert!(result.summary.contains("focus on auth"));
    }
}

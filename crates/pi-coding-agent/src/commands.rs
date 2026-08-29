use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
}

pub fn builtin_slash_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "settings".to_string(),
            description: "Open settings menu".to_string(),
            argument_hint: None,
        },
        SlashCommand {
            name: "model".to_string(),
            description: "Select model".to_string(),
            argument_hint: Some("<provider/model>".to_string()),
        },
        SlashCommand {
            name: "thinking".to_string(),
            description: "Set thinking level".to_string(),
            argument_hint: Some("<level>".to_string()),
        },
        SlashCommand {
            name: "export".to_string(),
            description: "Export session".to_string(),
            argument_hint: Some("<path>".to_string()),
        },
        SlashCommand {
            name: "compact".to_string(),
            description: "Manually compact the session context".to_string(),
            argument_hint: None,
        },
        SlashCommand {
            name: "resume".to_string(),
            description: "Resume a different session".to_string(),
            argument_hint: None,
        },
        SlashCommand {
            name: "quit".to_string(),
            description: "Quit pi".to_string(),
            argument_hint: None,
        },
    ]
}

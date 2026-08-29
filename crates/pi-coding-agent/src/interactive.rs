//! Interactive TUI: Editor + agent loop + slash commands.

use std::io::{BufRead, Write};
use std::path::Path;

use pi_agent::{Agent, AgentContext, AgentLoopConfig, QueueMode, ToolExecutionMode};
use pi_ai::{test_model, AssistantContent, Message, MockProvider};
use pi_session::{provision_message, SessionCreateOptions, SessionRepository};
use pi_session_sqlite::{SqliteSessionRepository, WriterLeaseOptions};
use pi_tui::{parse_slash, ChatView, Component, SlashCommand};

use crate::{create_coding_tools, list_sessions, with_cwd};

pub fn run_interactive(
    input: impl BufRead,
    mut output: impl Write,
    database: &Path,
) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    with_cwd(cwd.clone(), || {
        let context = AgentContext {
            system_prompt: Some("You are pi.".into()),
            messages: vec![],
            tools: create_coding_tools(cwd.clone()),
        };
        let config = AgentLoopConfig {
            model: test_model(),
            tool_execution: ToolExecutionMode::Sequential,
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        };
        let mut agent = Agent::new(config, MockProvider::default()).with_context(context);
        let mut view = ChatView {
            title: format!("pi {} — Rust default", crate::VERSION),
            status: "Type a prompt, or /help. /exit to quit.".into(),
            ..ChatView::default()
        };
        let mut store = SqliteSessionRepository::open(database, WriterLeaseOptions::default())
            .ok()
            .and_then(|mut repo| {
                repo.create(SessionCreateOptions {
                    cwd: cwd.to_string_lossy().into_owned(),
                    name: Some("interactive".into()),
                    ..SessionCreateOptions::default()
                })
                .ok()
            });

        writeln!(output, "{}", view.render(80).join("\n")).map_err(|error| error.to_string())?;

        for line in input.lines() {
            let line = line.map_err(|error| error.to_string())?;
            let submitted = line.trim();
            if submitted.is_empty() {
                continue;
            }
            match parse_slash(submitted) {
                Some(SlashCommand::Exit) => {
                    if let Some(session) = store.as_mut() {
                        let _ = session.release();
                    }
                    writeln!(output, "goodbye").map_err(|error| error.to_string())?;
                    return Ok(());
                }
                Some(SlashCommand::Help) => {
                    view.status =
                        "/exit  leave    /help  this text    /clear  reset    /sessions  list"
                            .into();
                }
                Some(SlashCommand::Clear) => {
                    view.history.clear();
                    view.status = "cleared".into();
                }
                Some(SlashCommand::Sessions) => {
                    let rows = list_sessions(database).unwrap_or_default();
                    view.status = if rows.is_empty() {
                        "no sessions".into()
                    } else {
                        rows.join(" | ")
                    };
                }
                Some(SlashCommand::Unknown(name)) => {
                    view.status = format!("unknown command /{name}");
                }
                None => {
                    view.push_user(submitted);
                    if let Some(session) = store.as_mut() {
                        let _ = session.append_entry(provision_message(submitted), "main");
                    }
                    let _events = agent.prompt(submitted);
                    let reply = last_assistant_text(agent.messages());
                    view.push_assistant(&reply);
                    view.status.clear();
                    view.editor = Default::default();
                }
            }
            writeln!(output, "{}", view.render(80).join("\n"))
                .map_err(|error| error.to_string())?;
        }
        if let Some(session) = store.as_mut() {
            let _ = session.release();
        }
        Ok(())
    })
}

pub fn run_interactive_lines(lines: &[&str], database: &Path) -> Result<String, String> {
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    run_interactive(input.as_bytes(), &mut output, database)?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

fn last_assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::Assistant { content, .. } => content.iter().find_map(|block| match block {
                AssistantContent::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
}

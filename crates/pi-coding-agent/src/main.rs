use clap::Parser;
use pi_agent::agent::Agent;
use pi_ai::auth::InMemoryCredentialStore;
use pi_ai::models::Models;
use pi_ai::types::Context;
use pi_coding_agent::args::CliArgs;
use pi_coding_agent::tools::{BashTool, EditTool, ReadTool, WriteTool};
use pi_session::manager::SessionManager;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();

    let credential_store = Arc::new(InMemoryCredentialStore::new());
    let models_manager = Models::new(credential_store);

    let default_model = models_manager
        .get_models(None)
        .into_iter()
        .next()
        .expect("At least one default model configured");

    let mut agent = Agent::new(default_model.clone());

    if !args.no_tools {
        agent.add_tool(Arc::new(ReadTool));
        agent.add_tool(Arc::new(WriteTool));
        agent.add_tool(Arc::new(EditTool));
        agent.add_tool(Arc::new(BashTool));
    }

    let session_dir = args
        .session_dir
        .unwrap_or_else(|| "/tmp/pi-sessions".to_string());
    let session_mgr = SessionManager::new(&session_dir);

    if args.print {
        let prompt_text = args.messages.join(" ");
        if prompt_text.is_empty() {
            println!("Pi Agent v0.84.4 - Ready.");
            return Ok(());
        }

        agent.prompt(&prompt_text);

        let context = Context {
            system_prompt: Some("You are Pi, an expert autonomous coding assistant.".to_string()),
            messages: vec![pi_ai::types::Message::User(pi_ai::types::UserMessage {
                role: "user".to_string(),
                content: vec![pi_ai::types::UserContent::Text(pi_ai::types::TextContent {
                    content_type: "text".to_string(),
                    text: prompt_text,
                    text_signature: None,
                })],
                timestamp: chrono::Utc::now().timestamp_millis(),
            })],
            tools: None,
        };

        let stream = models_manager.stream_simple(&default_model, &context, None);
        if let Some(msg) = stream.result().await {
            let text = pi_ai::utils::content_text(&msg.content);
            println!("{}", text);
        }
        return Ok(());
    }

    if args.mode.as_deref() == Some("rpc") {
        println!("{{\"type\":\"ready\",\"version\":\"0.84.4\"}}");
        return Ok(());
    }

    println!("Pi Coding Agent (Rust) v0.84.4");
    println!("Session directory: {}", session_dir);
    let _ = session_mgr.list_sessions();

    Ok(())
}

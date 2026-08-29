use clap::{Parser, Subcommand};
use pi_coding_agent::PrintSession;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "pi",
    about = "Pi coding agent (Rust port; TypeScript remains authoritative)"
)]
struct Cli {
    #[arg(short = 'p', long, help = "Print mode: run one prompt and exit")]
    print: bool,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    #[command(subcommand)]
    command: Option<Commands>,
    prompt: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    Prompt { text: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let text = match (&cli.command, &cli.prompt) {
        (Some(Commands::Prompt { text }), _) => text.clone(),
        (_, Some(text)) => text.clone(),
        _ => {
            if cli.print {
                anyhow::bail!("print mode requires a prompt");
            }
            println!("pi interactive TUI is available via the Rust crate; TypeScript remains the shipping interactive product until cutover.");
            return Ok(());
        }
    };
    let session = PrintSession::open(&cli.cwd, None)?;
    let reply = session.prompt(&text).await?;
    println!("{reply}");
    Ok(())
}

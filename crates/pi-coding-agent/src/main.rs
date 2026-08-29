use clap::Parser;
use pi_coding_agent::{list_sessions, run_print, Args, Commands};

fn main() {
    let args = Args::parse();
    let result = match args.command {
        Some(Commands::Sessions { database }) => {
            list_sessions(&database).map(|rows| rows.join("\n"))
        }
        None => {
            if let Some(prompt) = args.print {
                run_print(&prompt, args.json)
            } else {
                Ok("pi interactive TUI is available via pi-tui; use -p for print mode.".into())
            }
        }
    };
    match result {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
        }
        Err(error) => {
            eprintln!("pi: {error}");
            std::process::exit(1);
        }
    }
}

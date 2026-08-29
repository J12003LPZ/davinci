use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::Parser;
use pi_coding_agent::{list_sessions, run_interactive, run_print, run_rpc, Args, Commands};

fn main() {
    let args = Args::parse();
    let result = match args.command {
        Some(Commands::Sessions { database }) => {
            list_sessions(&database).map(|rows| rows.join("\n"))
        }
        None if args.is_rpc() => run_rpc(io::stdin().lock(), io::stdout()).map(|()| String::new()),
        None => {
            if let Some(ref prompt) = args.print {
                run_print(prompt, args.is_json())
            } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
                run_interactive(io::stdin().lock(), io::stdout(), &args.database)
                    .map(|()| String::new())
            } else if !io::stdin().is_terminal() {
                let mut prompt = String::new();
                if io::stdin().read_line(&mut prompt).is_ok() {
                    let prompt = prompt.trim();
                    if prompt.is_empty() {
                        Ok(String::new())
                    } else {
                        run_print(prompt, args.is_json())
                    }
                } else {
                    Ok(String::new())
                }
            } else {
                run_interactive(
                    io::stdin().lock(),
                    io::stdout(),
                    &PathBuf::from("sessions.sqlite"),
                )
                .map(|()| String::new())
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

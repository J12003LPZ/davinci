use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "pi", version = "0.84.4", about = "Pi coding agent CLI")]
pub struct CliArgs {
    #[arg(short = 'p', long)]
    pub print: bool,

    #[arg(long)]
    pub mode: Option<String>,

    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub thinking: Option<String>,

    #[arg(short = 'c', long = "continue")]
    pub continue_session: bool,

    #[arg(short = 'r', long = "resume")]
    pub resume: bool,

    #[arg(long = "session-dir")]
    pub session_dir: Option<String>,

    #[arg(long = "session")]
    pub session: Option<String>,

    #[arg(long = "no-session")]
    pub no_session: bool,

    #[arg(long = "no-tools")]
    pub no_tools: bool,

    #[arg(long = "export")]
    pub export: Option<String>,

    #[arg(trailing_var_arg = true)]
    pub messages: Vec<String>,
}

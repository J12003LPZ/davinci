use clap::Parser;
use davinci_parity::{diff_jsonl, maybe_parallel_run, run_all};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "pi-parity",
    about = "Golden fixtures and optional TypeScript pi diffs"
)]
struct Cli {
    #[arg(long)]
    parallel_run: bool,
    #[arg(long)]
    diff_jsonl: Option<PathBuf>,
    #[arg(long)]
    ts_bin: Option<PathBuf>,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let dir = tempfile_dir()?;
    let reports = run_all(&dir)?;
    for report in &reports {
        println!(
            "{}: {}",
            report.name,
            if report.passed { "ok" } else { "fail" }
        );
    }
    if let Some(path) = cli.diff_jsonl {
        let left = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
        println!("{}", diff_jsonl(&left, &left));
    }
    if cli.parallel_run {
        println!("{}", maybe_parallel_run(cli.ts_bin.as_deref(), "rust-ok"));
    }
    if reports.iter().all(|r| r.passed) {
        Ok(())
    } else {
        Err("parity corpora failed".into())
    }
}

fn tempfile_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("pi-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir)
}

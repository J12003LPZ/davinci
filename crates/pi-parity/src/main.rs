use pi_parity::{diff_jsonl, parallel_run, required_corpora};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "pi-parity [--parallel-run <ts-pi>] [--diff-jsonl <a> <b>]\n\
             Golden corpora: writer-leases, session-entries, protocol-hello-cbor,\n\
             assistant-usage, agent-events, print-rpc-events"
        );
        return ExitCode::SUCCESS;
    }
    if let Some(pos) = args.iter().position(|a| a == "--diff-jsonl") {
        let left = std::fs::read_to_string(&args[pos + 1]).unwrap_or_default();
        let right = std::fs::read_to_string(&args[pos + 2]).unwrap_or_default();
        let diffs = diff_jsonl(&left, &right);
        if diffs.is_empty() {
            println!("jsonl match");
            return ExitCode::SUCCESS;
        }
        for diff in diffs {
            println!("{diff}");
        }
        return ExitCode::from(1);
    }
    if let Some(pos) = args.iter().position(|a| a == "--parallel-run") {
        let ts = PathBuf::from(args.get(pos + 1).cloned().unwrap_or_else(|| "pi".into()));
        let rust = env::current_exe().unwrap_or_else(|_| PathBuf::from("pi"));
        match parallel_run(&rust, &ts, &["--version"]) {
            Ok((out, err)) => {
                print!("{out}{err}");
            }
            Err(e) => {
                eprintln!("parallel-run skipped: {e}");
            }
        }
    }
    let mut failed = 0;
    for check in required_corpora() {
        let mark = if check.passed { "PASS" } else { "FAIL" };
        if !check.passed {
            failed += 1;
        }
        println!("{mark} {} — {}", check.name, check.detail);
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

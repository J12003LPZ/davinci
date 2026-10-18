//! The startup trust question.
//!
//! Runs before the agent is built, so no project hook, MCP server, extension,
//! skill or package has loaded when it appears. Two answers only:
//! `No - Exit DaVinci` (the default) and `Yes, I trust this folder`. Both are
//! written to the same store `/trust` and `/setup trust` use.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use davinci_coding_agent::settings::load_settings;
use davinci_coding_agent::trust::{
    canonicalize_trust_path, preflight_project_trust, record_gate_choice,
    trust_requiring_project_resource_names, HostMode, TrustGateChoice, TrustGateDecision,
    TrustPolicy,
};

pub const LABEL_EXIT: &str = "No \u{2014} Exit DaVinci";
pub const LABEL_TRUST: &str = "Yes, I trust this folder";

/// What `run` does after the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    /// Carry on. Project resources load only if the store says trusted.
    Continue,
    /// Leave now with this exit code, having loaded nothing from the project.
    Exit(i32),
}

/// Read an answer typed on a line: `1`, `n`, `no`, or an empty line mean No;
/// `2`, `y`, `yes` mean Yes. Anything else is not an answer.
pub fn parse_answer(line: &str) -> Option<TrustGateChoice> {
    match line.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "n" | "no" => Some(TrustGateChoice::Exit),
        "2" | "y" | "yes" => Some(TrustGateChoice::Trust),
        _ => None,
    }
}

/// The question as plain lines (no cursor control), for a pipe or a terminal.
pub fn prompt_lines(path: &str, resources: &[String]) -> Vec<String> {
    let mut lines = vec![
        "Do you trust the files in this folder?".to_string(),
        format!("  {path}"),
        String::new(),
    ];
    if resources.is_empty() {
        lines.push("It has project configuration that can run code or change prompts.".into());
    } else {
        lines.push(format!(
            "It has project configuration that can run code or change prompts: {}.",
            resources.join(", ")
        ));
    }
    lines
        .push("Trusting only lets DaVinci load it. Tool approvals and the sandbox stay on.".into());
    lines.push(String::new());
    lines
}

fn option_line(index: usize, label: &str, selected: bool) -> String {
    let marker = if selected { ">" } else { " " };
    format!("{marker} {}. {label}", index + 1)
}

/// Ask on a pipe or redirected stdin: print the question, read one line per
/// attempt. End of input and unreadable input mean No, so a closed pipe never
/// grants trust.
pub fn ask_from_lines(
    input: &mut impl BufRead,
    output: &mut impl Write,
    path: &str,
    resources: &[String],
) -> TrustGateChoice {
    for line in prompt_lines(path, resources) {
        let _ = writeln!(output, "{line}");
    }
    let _ = writeln!(output, "{}", option_line(0, LABEL_EXIT, true));
    let _ = writeln!(output, "{}", option_line(1, LABEL_TRUST, false));
    for _ in 0..3 {
        let _ = write!(output, "Choose 1 or 2 [1]: ");
        let _ = output.flush();
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => return TrustGateChoice::Exit,
            Ok(_) => {}
        }
        if let Some(choice) = parse_answer(&line) {
            return choice;
        }
        let _ = writeln!(output, "Answer 1 (No) or 2 (Yes).");
    }
    TrustGateChoice::Exit
}

/// Ask on a terminal: arrow keys move, Enter confirms, `1`/`n` and `2`/`y`
/// answer at once, Esc and Ctrl+C mean No.
fn ask_on_terminal(path: &str, resources: &[String]) -> io::Result<TrustGateChoice> {
    use crossterm::cursor::MoveUp;
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::terminal::{self, Clear, ClearType};
    use crossterm::{queue, style::Print};

    struct Raw;
    impl Drop for Raw {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
        }
    }

    let mut out = io::stdout();
    let intro = prompt_lines(path, resources);
    for line in &intro {
        queue!(out, Print(line), Print("\r\n"))?;
    }
    terminal::enable_raw_mode()?;
    let _raw = Raw;
    let mut selected = 0usize;
    let mut first = true;
    loop {
        if !first {
            queue!(out, MoveUp(3), Clear(ClearType::FromCursorDown))?;
        }
        first = false;
        queue!(
            out,
            Print(option_line(0, LABEL_EXIT, selected == 0)),
            Print("\r\n"),
            Print(option_line(1, LABEL_TRUST, selected == 1)),
            Print("\r\n"),
            Print("Enter to confirm \u{b7} Esc to exit\r\n"),
        )?;
        out.flush()?;
        let Event::Key(key) = crossterm::event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'd') if ctrl => return Ok(TrustGateChoice::Exit),
            KeyCode::Esc | KeyCode::Char('1' | 'n' | 'N') => return Ok(TrustGateChoice::Exit),
            KeyCode::Char('2' | 'y' | 'Y') => return Ok(TrustGateChoice::Trust),
            KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::Char('j' | 'k') => {
                selected = 1 - selected;
            }
            KeyCode::Enter => {
                return Ok(if selected == 0 {
                    TrustGateChoice::Exit
                } else {
                    TrustGateChoice::Trust
                })
            }
            _ => {}
        }
    }
}

fn ask(path: &str, resources: &[String]) -> TrustGateChoice {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        // A terminal that cannot enter raw mode cannot show the question, so
        // the answer is No rather than a guess.
        return ask_on_terminal(path, resources).unwrap_or(TrustGateChoice::Exit);
    }
    let stdin = io::stdin();
    ask_from_lines(&mut stdin.lock(), &mut io::stdout(), path, resources)
}

/// Decide before anything project-controlled loads. `trust_override` is the
/// `--approve` / `--no-approve` flag; everything else comes from the trust
/// store and the global settings, never from the project.
pub fn enforce_startup_trust(
    agent_dir: &Path,
    cwd: &Path,
    trust_override: Option<bool>,
    mode: HostMode,
) -> GateOutcome {
    let settings = load_settings(agent_dir);
    let policy = TrustPolicy {
        override_trust: trust_override,
        default_project_trust: settings.default_project_trust.as_deref(),
        trusted_projects: &settings.trusted_projects,
    };
    let decision = preflight_project_trust(agent_dir, cwd, &policy, mode);
    crate::startup_mark("trust gate");
    match decision {
        TrustGateDecision::Preapproved | TrustGateDecision::Declined => GateOutcome::Continue,
        TrustGateDecision::StoreUnreadable(message) => {
            eprintln!(
                "Error: {message}\nProject resources stay unloaded until the trust store is readable."
            );
            GateOutcome::Continue
        }
        // Task 4 reports this to headless callers.
        TrustGateDecision::Unavailable => GateOutcome::Continue,
        TrustGateDecision::Prompt => {
            let path = canonicalize_trust_path(cwd);
            let resources = trust_requiring_project_resource_names(cwd);
            let choice = ask(&path, &resources);
            crate::startup_mark("trust gate answered");
            if let Err(err) = record_gate_choice(agent_dir, cwd, choice) {
                eprintln!(
                    "Error: could not save the answer: {err}\nNothing from {path} was loaded."
                );
                return GateOutcome::Exit(1);
            }
            match choice {
                TrustGateChoice::Trust => GateOutcome::Continue,
                TrustGateChoice::Exit => {
                    println!(
                        "\nNothing from {path} was loaded. Opening DaVinci there again starts it without the project's hooks, MCP servers, extensions and skills; /trust changes that."
                    );
                    GateOutcome::Exit(0)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn ask_with(input: &str) -> (TrustGateChoice, String) {
        let mut output = Vec::new();
        let choice = ask_from_lines(
            &mut Cursor::new(input.as_bytes().to_vec()),
            &mut output,
            "C:\\work\\repo",
            &["mcp.json".into(), "hooks.json".into()],
        );
        (choice, String::from_utf8(output).unwrap())
    }

    #[test]
    fn the_question_has_exactly_the_two_requested_options() {
        let (_, shown) = ask_with("\n");
        assert!(shown.contains("1. No \u{2014} Exit DaVinci"), "{shown}");
        assert!(shown.contains("2. Yes, I trust this folder"), "{shown}");
        assert!(shown.contains("C:\\work\\repo"));
        assert!(shown.contains("mcp.json, hooks.json"));
        assert!(
            !shown.contains("parent"),
            "no parent-folder option: {shown}"
        );
        assert!(
            !shown.contains("session"),
            "no session-only option: {shown}"
        );
    }

    #[test]
    fn no_is_the_default_and_closed_input_never_grants_trust() {
        assert_eq!(ask_with("\n").0, TrustGateChoice::Exit);
        assert_eq!(ask_with("").0, TrustGateChoice::Exit);
        assert_eq!(ask_with("1\n").0, TrustGateChoice::Exit);
        assert_eq!(
            ask_with("garbage\ngarbage\ngarbage\n2\n").0,
            TrustGateChoice::Exit
        );
    }

    #[test]
    fn yes_needs_an_explicit_answer() {
        for yes in ["2\n", "y\n", "Y\n", "yes\n", " Yes \n"] {
            assert_eq!(ask_with(yes).0, TrustGateChoice::Trust, "{yes:?}");
        }
        // A bad answer re-asks instead of guessing.
        assert_eq!(ask_with("maybe\n2\n").0, TrustGateChoice::Trust);
    }

    #[test]
    fn answers_parse_case_and_whitespace_insensitively() {
        assert_eq!(parse_answer("  NO "), Some(TrustGateChoice::Exit));
        assert_eq!(parse_answer("YES"), Some(TrustGateChoice::Trust));
        assert_eq!(parse_answer("trust"), None);
    }
}

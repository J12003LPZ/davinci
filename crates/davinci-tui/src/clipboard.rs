//! Put text on the system clipboard, by whichever path reaches it.
//!
//! Local sessions run the platform's own tool: `pbcopy`; `wl-copy` on
//! Wayland or `xclip`/`xsel` on X11, writing both CLIPBOARD and PRIMARY so a
//! middle click pastes too; PowerShell `Set-Clipboard` on Windows and WSL,
//! fed UTF-8 so non-ASCII text survives (`clip.exe` stays as a last resort).
//! Inside tmux the text also goes into tmux's paste buffer. Over SSH no local
//! tool can reach the user's machine, so the text travels to the terminal as
//! an OSC 52 sequence, wrapped for tmux or GNU screen when one sits between.
//! Every copy reports the path it took; a failure says so instead of
//! claiming the clipboard holds the text.

use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use base64::Engine;

/// The facts that decide the route, read once per copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardEnv {
    pub os: Os,
    /// `SSH_TTY` or `SSH_CONNECTION`: local tools would fill the remote host's
    /// clipboard, not the user's.
    pub ssh: bool,
    pub tmux: bool,
    /// GNU screen (`STY`).
    pub screen: bool,
    pub wayland: bool,
    pub x11: bool,
    pub wsl: bool,
    /// VTE-based terminals ignore OSC 52, and some versions print it.
    pub vte: bool,
    /// Whether stdout reaches a terminal at all; OSC 52 needs one.
    pub tty: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Os {
    #[default]
    Linux,
    Mac,
    Windows,
}

/// The terminal multiplexer an OSC 52 sequence has to pass through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mux {
    None,
    Tmux,
    Screen,
}

/// One way of reaching a clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Run `program args…` with the text on stdin.
    Run {
        program: &'static str,
        args: Vec<&'static str>,
        label: &'static str,
    },
    /// Write OSC 52 to the terminal.
    Osc52(Mux),
}

impl ClipboardEnv {
    pub fn detect() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let wsl = os == Os::Linux
            && (set("WSL_DISTRO_NAME")
                || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                    .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft")));
        Self {
            os,
            ssh: set("SSH_TTY") || set("SSH_CONNECTION"),
            tmux: set("TMUX"),
            screen: set("STY"),
            wayland: set("WAYLAND_DISPLAY"),
            x11: set("DISPLAY"),
            wsl,
            vte: set("VTE_VERSION"),
            tty: std::io::stdout().is_terminal(),
        }
    }

    fn mux(&self) -> Mux {
        if self.tmux {
            Mux::Tmux
        } else if self.screen {
            Mux::Screen
        } else {
            Mux::None
        }
    }

    fn osc52_reaches_terminal(&self) -> bool {
        self.tty && !self.vte
    }
}

const POWERSHELL_SET: &str = "[Console]::InputEncoding=[Text.Encoding]::UTF8;\
     Set-Clipboard -Value ([Console]::In.ReadToEnd())";

/// The ways to try, best first. A step whose program is not installed is
/// skipped by [`copy`]; the first that succeeds is the one reported.
pub fn plan(env: &ClipboardEnv) -> Vec<Step> {
    let run = |program, args: &[&'static str], label| Step::Run {
        program,
        args: args.to_vec(),
        label,
    };
    let mut steps = Vec::new();
    if env.ssh {
        if env.osc52_reaches_terminal() {
            steps.push(Step::Osc52(env.mux()));
        }
        return steps;
    }
    match env.os {
        Os::Mac => steps.push(run("pbcopy", &[], "pbcopy")),
        Os::Windows => {
            steps.push(run(
                "powershell",
                &["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_SET],
                "Set-Clipboard",
            ));
            steps.push(run("clip", &[], "clip"));
        }
        Os::Linux => {
            if env.wayland {
                steps.push(run("wl-copy", &[], "wl-copy"));
            }
            if env.x11 {
                steps.push(run("xclip", &["-selection", "clipboard"], "xclip"));
                steps.push(run("xsel", &["--clipboard", "--input"], "xsel"));
            }
            if env.wsl {
                steps.push(run(
                    "powershell.exe",
                    &["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_SET],
                    "Set-Clipboard",
                ));
                steps.push(run("clip.exe", &[], "clip.exe"));
            }
        }
    }
    if env.osc52_reaches_terminal() {
        steps.push(Step::Osc52(env.mux()));
    }
    steps
}

/// The PRIMARY selection that goes with a successful CLIPBOARD write, so a
/// middle click pastes what was just copied.
fn primary_companion(label: &str) -> Option<(&'static str, Vec<&'static str>)> {
    match label {
        "wl-copy" => Some(("wl-copy", vec!["--primary"])),
        "xclip" => Some(("xclip", vec!["-selection", "primary"])),
        "xsel" => Some(("xsel", vec!["--primary", "--input"])),
        _ => None,
    }
}

/// OSC 52 for `text`, wrapped so it passes through `mux`.
pub fn osc52(text: &str, mux: Mux) -> String {
    let payload = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    match mux {
        Mux::None => format!("\x1b]52;c;{payload}\x07"),
        // tmux forwards a DCS passthrough whose inner escapes are doubled.
        Mux::Tmux => format!("\x1bPtmux;\x1b\x1b]52;c;{payload}\x07\x1b\\"),
        // screen limits a DCS string, so the sequence is split into chunks,
        // each its own passthrough.
        Mux::Screen => {
            let sequence = format!("\x1b]52;c;{payload}\x07");
            let mut out = String::new();
            let bytes = sequence.as_bytes();
            for chunk in bytes.chunks(76) {
                out.push_str("\x1bP");
                out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
                out.push_str("\x1b\\");
            }
            out
        }
    }
}

fn run_with_stdin(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("could not run {program}: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|err| format!("could not write to {program}: {err}"))?;
    }
    let status = child
        .wait()
        .map_err(|err| format!("could not wait for {program}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

/// Copy `text`, returning the path that took it (`wl-copy`, `OSC 52`, …).
pub fn copy(text: &str) -> Result<String, String> {
    copy_with(&ClipboardEnv::detect(), text, &mut std::io::stdout())
}

/// [`copy`] with the environment and terminal supplied.
pub fn copy_with(
    env: &ClipboardEnv,
    text: &str,
    terminal: &mut impl Write,
) -> Result<String, String> {
    let mut last_error = None;
    let mut done = None;
    for step in plan(env) {
        match step {
            Step::Run {
                program,
                args,
                label,
            } => match run_with_stdin(program, &args, text) {
                Ok(()) => {
                    if let Some((program, args)) = primary_companion(label) {
                        let _ = run_with_stdin(program, &args, text);
                    }
                    done = Some(label.to_string());
                    break;
                }
                Err(err) => last_error = Some(err),
            },
            Step::Osc52(mux) => {
                terminal
                    .write_all(osc52(text, mux).as_bytes())
                    .and_then(|()| terminal.flush())
                    .map_err(|err| format!("could not write OSC 52: {err}"))?;
                done = Some("OSC 52".to_string());
                break;
            }
        }
    }
    // The tmux buffer is extra, never the only copy: `prefix ]` pastes it.
    if env.tmux && !env.ssh && run_with_stdin("tmux", &["load-buffer", "-"], text).is_ok() {
        done = Some(match done {
            Some(label) => format!("{label} + tmux buffer"),
            None => "tmux buffer".into(),
        });
    }
    done.ok_or_else(|| last_error.unwrap_or_else(|| "no clipboard tool is available".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(env: &ClipboardEnv) -> Vec<String> {
        plan(env)
            .into_iter()
            .map(|step| match step {
                Step::Run { label, .. } => label.to_string(),
                Step::Osc52(mux) => format!("osc52:{mux:?}"),
            })
            .collect()
    }

    #[test]
    fn ssh_goes_through_the_terminal_wrapped_for_the_multiplexer() {
        let env = ClipboardEnv {
            ssh: true,
            tmux: true,
            x11: true,
            tty: true,
            ..ClipboardEnv::default()
        };
        assert_eq!(labels(&env), vec!["osc52:Tmux"]);
        let env = ClipboardEnv { tty: false, ..env };
        assert!(labels(&env).is_empty(), "no terminal, no OSC 52");
    }

    #[test]
    fn local_linux_prefers_wayland_then_x11_then_the_terminal() {
        let env = ClipboardEnv {
            wayland: true,
            x11: true,
            tty: true,
            ..ClipboardEnv::default()
        };
        assert_eq!(labels(&env), vec!["wl-copy", "xclip", "xsel", "osc52:None"]);
        let vte = ClipboardEnv { vte: true, ..env };
        assert_eq!(labels(&vte), vec!["wl-copy", "xclip", "xsel"]);
    }

    #[test]
    fn windows_and_wsl_use_set_clipboard_before_clip() {
        let env = ClipboardEnv {
            os: Os::Windows,
            ..ClipboardEnv::default()
        };
        assert_eq!(labels(&env), vec!["Set-Clipboard", "clip"]);
        let env = ClipboardEnv {
            wsl: true,
            ..ClipboardEnv::default()
        };
        assert_eq!(labels(&env), vec!["Set-Clipboard", "clip.exe"]);
    }

    #[test]
    fn osc52_is_wrapped_for_tmux_and_chunked_for_screen() {
        assert_eq!(osc52("hi", Mux::None), "\x1b]52;c;aGk=\x07");
        assert_eq!(
            osc52("hi", Mux::Tmux),
            "\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\"
        );
        let long = "x".repeat(200);
        let screen = osc52(&long, Mux::Screen);
        let chunks = screen.matches("\x1bP").count();
        assert!(chunks > 1, "{chunks}");
        let joined: String = screen
            .split("\x1b\\")
            .map(|part| part.trim_start_matches("\x1bP"))
            .collect();
        assert_eq!(joined, osc52(&long, Mux::None));
    }

    #[test]
    fn a_terminal_copy_reports_osc52_and_writes_it() {
        let env = ClipboardEnv {
            ssh: true,
            tty: true,
            ..ClipboardEnv::default()
        };
        let mut out = Vec::new();
        assert_eq!(copy_with(&env, "hi", &mut out).as_deref(), Ok("OSC 52"));
        assert_eq!(out, b"\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn nothing_reachable_is_an_error_not_a_claim() {
        let env = ClipboardEnv {
            ssh: true,
            ..ClipboardEnv::default()
        };
        assert!(copy_with(&env, "hi", &mut Vec::new()).is_err());
    }
}

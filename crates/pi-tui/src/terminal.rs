use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiMode {
    Regular,
    Fullscreen,
}

impl TuiMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "regular" => Some(Self::Regular),
            "fullscreen" => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

pub fn enter_alt_screen(out: &mut impl Write) -> io::Result<()> {
    write!(out, "\u{1b}[?1049h\u{1b}[?25l")
}

pub fn leave_alt_screen(out: &mut impl Write) -> io::Result<()> {
    write!(out, "\u{1b}[?25h\u{1b}[?1049l")
}

pub fn enable_mouse(out: &mut impl Write) -> io::Result<()> {
    write!(out, "\u{1b}[?1000h\u{1b}[?1006h")
}

pub fn disable_mouse(out: &mut impl Write) -> io::Result<()> {
    write!(out, "\u{1b}[?1006l\u{1b}[?1000l")
}

/// Enable/disable cooked canonical input via `stty` (no `unsafe`).
pub fn enable_raw_input() -> io::Result<()> {
    let status = std::process::Command::new("stty")
        .args(["-icanon", "-echo", "min", "1", "time", "0"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("stty raw failed"))
    }
}

pub fn disable_raw_input() -> io::Result<()> {
    let status = std::process::Command::new("stty")
        .args(["icanon", "echo"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("stty cooked failed"))
    }
}

pub fn enter_raw_mode() -> io::Result<()> {
    enable_raw_input()
}

pub fn leave_raw_mode() -> io::Result<()> {
    disable_raw_input()
}

/// OSC 0 window title, matching TypeScript `terminal.setTitle`.
pub fn set_title(out: &mut impl Write, title: &str) -> io::Result<()> {
    let cleaned: String = title
        .chars()
        .filter(|c| *c != '\u{1b}' && *c != '\u{07}')
        .collect();
    write!(out, "\u{1b}]0;{cleaned}\u{07}")
}

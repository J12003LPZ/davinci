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

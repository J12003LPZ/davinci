//! Named colours as the reference writes them.
//!
//! crossterm writes every named colour as an indexed one (`38;5;1`), where the
//! reference writes `31`. At ANSI depth with `tui.reference` on, this adapter
//! sits between the backend and stdout and rewrites exactly those parameters
//! back to the named forms, leaving everything else alone.
use std::io::{self, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

const ESC: u8 = 0x1b;

/// Where the byte stream is. A sequence can arrive in as many `write` calls
/// as crossterm cares to make, so the state outlives each call.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// After `ESC`, `held` has it.
    Esc,
    /// Inside `ESC [ ...`, `held` has what came so far.
    Csi,
    /// Inside an `ESC ] ...` string (hyperlinks): passed through untouched.
    Osc,
    /// `ESC` inside such a string; `\` ends it.
    OscEsc,
    /// Inside a DCS, APC, PM or SOS string: opaque like an OSC, but only the
    /// string terminator (`ESC \`) ends it.
    Str,
    /// `ESC` inside one of those strings.
    StrEsc,
}

/// Wraps the writer frames go through. The switch is shared with the session,
/// which sets it before each frame from the theme then in force; it is read
/// as each sequence starts, so a switch never cuts one in two.
pub struct NamedSgr<W: Write> {
    inner: W,
    on: Arc<AtomicBool>,
    state: State,
    /// A sequence begun while the switch was on.
    rewriting: bool,
    held: Vec<u8>,
}

impl<W: Write> NamedSgr<W> {
    pub fn new(inner: W, on: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            on,
            state: State::Ground,
            rewriting: false,
            held: Vec::new(),
        }
    }

    /// A byte outside any sequence.
    fn ground(&mut self, byte: u8, out: &mut Vec<u8>) {
        if byte == ESC {
            self.rewriting = self.on.load(Ordering::SeqCst);
            self.held.clear();
            self.held.push(byte);
            self.state = State::Esc;
        } else {
            out.push(byte);
        }
    }

    /// Give up on the held bytes: they were not a sequence we know.
    fn release(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.held);
        self.state = State::Ground;
    }

    fn feed(&mut self, buf: &[u8], out: &mut Vec<u8>) {
        for &byte in buf {
            match self.state {
                State::Ground => self.ground(byte, out),
                State::Esc => match byte {
                    b'[' => {
                        self.held.push(byte);
                        self.state = State::Csi;
                    }
                    b']' => {
                        self.release(out);
                        out.push(byte);
                        self.state = State::Osc;
                    }
                    // DCS, SOS, PM and APC.
                    b'P' | b'X' | b'^' | b'_' => {
                        self.release(out);
                        out.push(byte);
                        self.state = State::Str;
                    }
                    _ => {
                        self.release(out);
                        self.ground(byte, out);
                    }
                },
                State::Csi => match byte {
                    // Parameter and intermediate bytes.
                    0x20..=0x3f => self.held.push(byte),
                    // The final byte.
                    0x40..=0x7e => {
                        self.held.push(byte);
                        self.finish(out);
                    }
                    _ => {
                        self.release(out);
                        self.ground(byte, out);
                    }
                },
                State::Osc => {
                    out.push(byte);
                    match byte {
                        0x07 => self.state = State::Ground,
                        ESC => self.state = State::OscEsc,
                        _ => {}
                    }
                }
                State::OscEsc => {
                    out.push(byte);
                    self.state = if byte == b'\\' {
                        State::Ground
                    } else {
                        State::Osc
                    };
                }
                State::Str => {
                    out.push(byte);
                    if byte == ESC {
                        self.state = State::StrEsc;
                    }
                }
                State::StrEsc => {
                    out.push(byte);
                    self.state = if byte == b'\\' {
                        State::Ground
                    } else {
                        State::Str
                    };
                }
            }
        }
    }

    /// Write out a partial sequence as it came and return to rest.
    fn write_held(&mut self) -> io::Result<()> {
        if self.state == State::Ground || self.held.is_empty() {
            return Ok(());
        }
        let held = std::mem::take(&mut self.held);
        self.state = State::Ground;
        self.inner.write_all(&held)
    }

    /// A whole `ESC [ params final`: rewritten if it is plain SGR, else as it came.
    fn finish(&mut self, out: &mut Vec<u8>) {
        let last = self.held.len() - 1;
        let params = &self.held[2..last];
        let plain =
            self.held[last] == b'm' && params.iter().all(|b| b.is_ascii_digit() || *b == b';');
        if self.rewriting && plain {
            out.extend_from_slice(b"\x1b[");
            out.extend_from_slice(&named(params));
            out.push(b'm');
            self.held.clear();
            self.state = State::Ground;
        } else {
            self.release(out);
        }
    }
}

/// SGR parameters with `38;5;N` and `48;5;N` (N below 16) as named colours.
/// Truecolor triples and the underline colour are skipped whole, so their
/// numbers are never mistaken for the start of another colour.
fn named(params: &[u8]) -> Vec<u8> {
    let tokens: Vec<&[u8]> = params.split(|b| *b == b';').collect();
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let token = tokens[at];
        let base: Option<u16> = match token {
            b"38" => Some(30),
            b"48" => Some(40),
            _ => None,
        };
        let extended = matches!(token, b"38" | b"48" | b"58");
        let skip = match (extended, tokens.get(at + 1).copied()) {
            (true, Some(b"5")) => 3,
            (true, Some(b"2")) => 5,
            _ => 1,
        };
        let index = (skip == 3)
            .then(|| tokens.get(at + 2))
            .flatten()
            .filter(|t| !t.is_empty())
            .and_then(|t| std::str::from_utf8(t).ok()?.parse::<u16>().ok());
        match (base, index) {
            (Some(base), Some(n)) if n < 16 => {
                // 0-7 are 30/40 plus N; 8-15 are the bright 90/100 plus N-8.
                let code = if n < 8 { base + n } else { base + 60 + n - 8 };
                out.push(code.to_string().into_bytes());
                at += 3;
            }
            _ => {
                let end = (at + skip).min(tokens.len());
                out.extend(tokens[at..end].iter().map(|t| t.to_vec()));
                at = end;
            }
        }
    }
    out.join(&b';')
}

impl<W: Write> Write for NamedSgr<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Off, with nothing held: there is nothing to rewrite.
        if self.state == State::Ground && !self.on.load(Ordering::SeqCst) {
            return self.inner.write(buf);
        }
        let mut out = Vec::with_capacity(buf.len());
        self.feed(buf, &mut out);
        self.inner.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_held()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for NamedSgr<W> {
    fn drop(&mut self) {
        let _ = self.write_held();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        backend::CrosstermBackend,
        style::{Color, Modifier, Style},
        text::Span,
        widgets::Paragraph,
        Terminal,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A writer that keeps every `write` call as its own chunk.
    #[derive(Clone, Default)]
    struct Chunks(Rc<RefCell<Vec<Vec<u8>>>>);

    impl Write for Chunks {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn switch(on: bool) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(on))
    }

    /// Feed `pieces` as separate `write` calls and return what came out.
    fn through(on: bool, pieces: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = NamedSgr::new(&mut out, switch(on));
            for piece in pieces {
                writer.write_all(piece).unwrap();
            }
            writer.flush().unwrap();
        }
        out
    }

    #[test]
    fn measure_what_crossterm_emits_for_named_colours() {
        let chunks = Chunks::default();
        // A fixed viewport: there is no tty to ask for a size.
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(chunks.clone()),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 12, 2)),
            },
        )
        .unwrap();
        terminal
            .draw(|frame| {
                let style = Style::default()
                    .fg(Color::Red)
                    .bg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD | Modifier::DIM);
                let line = ratatui::text::Line::from(vec![
                    Span::styled("ab", style),
                    Span::styled("cd", Style::default().fg(Color::Indexed(200))),
                    Span::styled("ef", Style::default().fg(Color::Rgb(1, 2, 3))),
                    Span::styled("gh", Style::default().fg(Color::Gray).bg(Color::DarkGray)),
                ]);
                frame.render_widget(Paragraph::new(line), frame.area());
            })
            .unwrap();
        let chunks = chunks.0.borrow();
        let text: Vec<String> = chunks
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk).replace('\x1b', "ESC"))
            .collect();
        println!("{} write calls: {text:?}", chunks.len());
    }

    #[test]
    fn a_real_frame_comes_out_with_named_colours() {
        let chunks = Chunks::default();
        let on = switch(true);
        let backend = CrosstermBackend::new(NamedSgr::new(chunks.clone(), on.clone()));
        let mut terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 8, 1)),
            },
        )
        .unwrap();
        let line = ratatui::text::Line::from(vec![
            Span::styled("ab", Style::default().fg(Color::Red).bg(Color::LightBlue)),
            Span::styled("cd", Style::default().fg(Color::Indexed(200))),
            Span::styled("ef", Style::default().fg(Color::Rgb(1, 2, 3))),
        ]);
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(line.clone()), frame.area()))
            .unwrap();
        let bytes: Vec<u8> = chunks.0.borrow().iter().flatten().copied().collect();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\x1b[31;104m"), "{text:?}");
        assert!(text.contains("\x1b[38;5;200;49m"), "{text:?}");
        assert!(text.contains("\x1b[38;2;1;2;3;49m"), "{text:?}");
        assert!(!text.contains("38;5;1;"), "{text:?}");
    }

    #[test]
    fn named_colours_become_named_sgr_when_on() {
        assert_eq!(through(true, &[b"\x1b[38;5;1m"]), b"\x1b[31m");
        assert_eq!(through(true, &[b"\x1b[38;5;7m"]), b"\x1b[37m");
        assert_eq!(through(true, &[b"\x1b[38;5;8m"]), b"\x1b[90m");
        assert_eq!(through(true, &[b"\x1b[38;5;15m"]), b"\x1b[97m");
        assert_eq!(through(true, &[b"\x1b[48;5;0m"]), b"\x1b[40m");
        assert_eq!(through(true, &[b"\x1b[48;5;7m"]), b"\x1b[47m");
        assert_eq!(through(true, &[b"\x1b[48;5;9m"]), b"\x1b[101m");
        assert_eq!(through(true, &[b"\x1b[48;5;15m"]), b"\x1b[107m");
    }

    #[test]
    fn combined_parameters_keep_their_neighbours() {
        assert_eq!(through(true, &[b"\x1b[0;38;5;1m"]), b"\x1b[0;31m");
        assert_eq!(
            through(true, &[b"\x1b[1;38;5;9;48;5;4;2m"]),
            b"\x1b[1;91;44;2m"
        );
        assert_eq!(through(true, &[b"\x1b[38;5;1;38;5;2m"]), b"\x1b[31;32m");
    }

    #[test]
    fn a_sequence_split_across_writes_is_still_rewritten() {
        // crossterm writes the introducer, the numbers and the final byte apart.
        let pieces: [&[u8]; 5] = [b"\x1b[", b"38;5;", b"1", b"m", b"text"];
        assert_eq!(through(true, &pieces), b"\x1b[31mtext");
        for cut in 1..b"\x1b[0;48;5;12m".len() {
            let whole = b"\x1b[0;48;5;12m";
            let (head, tail) = whole.split_at(cut);
            assert_eq!(through(true, &[head, tail]), b"\x1b[0;104m", "cut at {cut}");
        }
    }

    #[test]
    fn only_indexes_below_sixteen_are_named() {
        for kept in [
            &b"\x1b[38;5;16m"[..],
            b"\x1b[38;5;200m",
            b"\x1b[48;5;255m",
            b"\x1b[38;5;256m",
        ] {
            assert_eq!(through(true, &[kept]), kept);
        }
    }

    #[test]
    fn truecolor_and_underline_colour_pass_through() {
        for kept in [
            &b"\x1b[38;2;1;2;3m"[..],
            b"\x1b[48;2;5;5;5m",
            // The `5` and `1` inside a truecolor triple are not an index.
            b"\x1b[38;2;5;5;1m",
            b"\x1b[38;2;38;5;1m",
            b"\x1b[58;5;1m",
            b"\x1b[58;2;1;2;3m",
        ] {
            assert_eq!(through(true, &[kept]), kept, "{kept:?}");
        }
        assert_eq!(
            through(true, &[b"\x1b[38;2;5;5;1;38;5;1m"]),
            b"\x1b[38;2;5;5;1;31m"
        );
    }

    #[test]
    fn nothing_changes_while_off() {
        let all = b"\x1b[38;5;1m\x1b[48;5;9mtext\x1b[0m";
        assert_eq!(through(false, &[all]), all);
    }

    #[test]
    fn text_that_only_looks_like_a_sequence_is_untouched() {
        for kept in [
            &b"38;5;1m"[..],
            b"[38;5;1m",
            b"a [38;5;1m b",
            b"ESC[38;5;1m",
            b"\x1b[?38;5;1m",
            b"\x1b[38;5;1 m",
            b"\x1b[38;5;1H",
            b"\x1b[38:5:1m",
        ] {
            assert_eq!(through(true, &[kept]), kept, "{kept:?}");
        }
        // Text after a finished sequence that merely resembles one.
        assert_eq!(through(true, &[b"\x1b[0m[38;5;1m"]), b"\x1b[0m[38;5;1m");
    }

    #[test]
    fn an_unfinished_sequence_followed_by_text_is_passed_whole() {
        assert_eq!(
            through(true, &[b"\x1b[38;5;", b"hello"]),
            b"\x1b[38;5;hello"
        );
        assert_eq!(through(true, &[b"\x1b", b"x38;5;1m"]), b"\x1bx38;5;1m");
        assert_eq!(
            through(true, &[b"\x1b[38;5;\x1b[38;5;2m"]),
            b"\x1b[38;5;\x1b[32m"
        );
    }

    #[test]
    fn hyperlink_sequences_pass_through() {
        let link = b"\x1b]8;;https://example.test/a?x=38;5;1m\x1b\\\x1b[38;5;1mtext\x1b]8;;\x1b\\";
        assert_eq!(
            through(true, &[link]),
            b"\x1b]8;;https://example.test/a?x=38;5;1m\x1b\\\x1b[31mtext\x1b]8;;\x1b\\"
        );
        // An OSC whose payload holds an escape-looking run stays whole, split or not.
        let osc: [&[u8]; 3] = [b"\x1b]8;;http://a/\x1b[38;5", b";1m", b"\x07\x1b[38;5;2m"];
        assert_eq!(
            through(true, &osc),
            b"\x1b]8;;http://a/\x1b[38;5;1m\x07\x1b[32m"
        );
    }

    /// Accepts at most two bytes per call.
    struct Short(Vec<u8>);

    impl Write for Short {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let n = buf.len().min(2);
            self.0.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn off_and_idle_the_bytes_go_straight_to_the_inner_writer() {
        let mut writer = NamedSgr::new(Short(Vec::new()), switch(false));
        assert_eq!(writer.write(b"abcd").unwrap(), 2, "the inner count");
        assert_eq!(writer.inner.0, b"ab");
        // Escapes do not change that, and nothing is held back.
        assert_eq!(writer.write(b"\x1b[38;5;1m").unwrap(), 2);
        assert_eq!(writer.inner.0, b"ab\x1b[");
    }

    #[test]
    fn flush_writes_a_held_partial_sequence_unchanged() {
        let mut out = Vec::new();
        let mut writer = NamedSgr::new(&mut out, switch(true));
        writer.write_all(b"ab\x1b[38;5;").unwrap();
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(out, b"ab\x1b[38;5;");
        // The rest is plain text now; the stream is back at rest.
        let mut out = Vec::new();
        let mut writer = NamedSgr::new(&mut out, switch(true));
        writer.write_all(b"\x1b[38;5;").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"1m\x1b[38;5;1m").unwrap();
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(out, b"\x1b[38;5;1m\x1b[31m");
    }

    #[test]
    fn dropping_the_adapter_writes_what_it_held() {
        let mut out = Vec::new();
        {
            let mut writer = NamedSgr::new(&mut out, switch(true));
            writer.write_all(b"\x1b").unwrap();
        }
        assert_eq!(out, b"\x1b");
    }

    #[test]
    fn device_control_and_application_strings_are_opaque() {
        for (open, name) in [
            (&b"\x1bP"[..], "DCS"),
            (b"\x1b_", "APC"),
            (b"\x1b^", "PM"),
            (b"\x1bX", "SOS"),
        ] {
            let mut wire = open.to_vec();
            wire.extend_from_slice(b"q\x1b[38;5;1m\x07more\x1b\\\x1b[38;5;1m");
            let mut expected = open.to_vec();
            expected.extend_from_slice(b"q\x1b[38;5;1m\x07more\x1b\\\x1b[31m");
            assert_eq!(through(true, &[&wire]), expected, "{name}");
            // Split anywhere, the payload is still left alone.
            for cut in 1..wire.len() {
                let (head, tail) = wire.split_at(cut);
                assert_eq!(through(true, &[head, tail]), expected, "{name} cut {cut}");
            }
        }
    }

    #[test]
    fn a_live_switch_applies_from_the_next_sequence() {
        let on = switch(false);
        let mut out = Vec::new();
        {
            let mut writer = NamedSgr::new(&mut out, on.clone());
            writer.write_all(b"\x1b[38;5;1m").unwrap();
            on.store(true, Ordering::SeqCst);
            writer.write_all(b"\x1b[38;5;1m").unwrap();
            on.store(false, Ordering::SeqCst);
            writer.write_all(b"\x1b[38;5;1m").unwrap();
        }
        assert_eq!(out, b"\x1b[38;5;1m\x1b[31m\x1b[38;5;1m");
    }
}

//! StdinBuffer matching `vendor/pi/packages/tui/src/stdin-buffer.ts`.
//!
//! Timeouts are driven by [`StdinBuffer::tick`] so tests stay deterministic.

const ESC: &str = "\x1b";
const DEFAULT_SEQUENCE_TIMEOUT_MS: u64 = 50;
const DEFAULT_ESCAPE_TIMEOUT_MS: u64 = 10;
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceStatus {
    Complete,
    Incomplete,
    NotEscape,
}

#[derive(Debug, Clone)]
pub struct StdinBufferOptions {
    pub timeout: u64,
    pub escape_timeout: u64,
}

impl Default for StdinBufferOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_SEQUENCE_TIMEOUT_MS,
            escape_timeout: DEFAULT_ESCAPE_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StdinEvents {
    pub data: Vec<String>,
    pub paste: Vec<String>,
}

impl StdinEvents {
    fn extend(&mut self, other: StdinEvents) {
        self.data.extend(other.data);
        self.paste.extend(other.paste);
    }
}

#[derive(Debug, Clone)]
pub struct StdinBuffer {
    buffer: String,
    timeout_ms: u64,
    escape_timeout_ms: u64,
    paste_mode: bool,
    paste_buffer: String,
    pending_kitty_printable_codepoint: Option<u32>,
    clock_ms: u64,
    timeout_at: Option<u64>,
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self::with_options(StdinBufferOptions::default())
    }

    pub fn with_options(options: StdinBufferOptions) -> Self {
        Self {
            buffer: String::new(),
            timeout_ms: options.timeout,
            escape_timeout_ms: options.escape_timeout,
            paste_mode: false,
            paste_buffer: String::new(),
            pending_kitty_printable_codepoint: None,
            clock_ms: 0,
            timeout_at: None,
        }
    }

    pub fn process(&mut self, data: &str) -> StdinEvents {
        self.process_bytes(data.as_bytes())
    }

    pub fn process_bytes(&mut self, data: &[u8]) -> StdinEvents {
        self.timeout_at = None;
        let str = if data.len() == 1 && data[0] > 127 {
            format!("\x1b{}", char::from(data[0] - 128))
        } else {
            String::from_utf8_lossy(data).into_owned()
        };
        if str.is_empty() && self.buffer.is_empty() {
            return StdinEvents {
                data: vec![String::new()],
                paste: Vec::new(),
            };
        }
        self.buffer.push_str(&str);
        self.drain_ready()
    }

    pub fn tick(&mut self, ms: u64) -> StdinEvents {
        self.clock_ms = self.clock_ms.saturating_add(ms);
        if self.timeout_at.is_some_and(|at| self.clock_ms >= at) {
            self.timeout_at = None;
            let flushed = self.flush();
            let mut events = StdinEvents::default();
            for sequence in flushed {
                self.emit_data_sequence(&mut events, sequence);
            }
            return events;
        }
        StdinEvents::default()
    }

    pub fn flush(&mut self) -> Vec<String> {
        self.timeout_at = None;
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let sequences = vec![std::mem::take(&mut self.buffer)];
        self.pending_kitty_printable_codepoint = None;
        sequences
    }

    pub fn clear(&mut self) {
        self.timeout_at = None;
        self.buffer.clear();
        self.paste_mode = false;
        self.paste_buffer.clear();
        self.pending_kitty_printable_codepoint = None;
    }

    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    pub fn destroy(&mut self) {
        self.clear();
    }

    fn drain_ready(&mut self) -> StdinEvents {
        let mut events = StdinEvents::default();
        if self.paste_mode {
            self.paste_buffer.push_str(&self.buffer);
            self.buffer.clear();
            if let Some(end_index) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let pasted_content = self.paste_buffer[..end_index].to_string();
                let remaining =
                    self.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                self.pending_kitty_printable_codepoint = None;
                events.paste.push(pasted_content);
                if !remaining.is_empty() {
                    events.extend(self.process(&remaining));
                }
            }
            return events;
        }

        if let Some(start_index) = self.buffer.find(BRACKETED_PASTE_START) {
            if start_index > 0 {
                let before_paste = self.buffer[..start_index].to_string();
                let result = extract_complete_sequences(&before_paste);
                for sequence in result.sequences {
                    self.emit_data_sequence(&mut events, sequence);
                }
            }
            self.pending_kitty_printable_codepoint = None;
            self.buffer = self.buffer[start_index + BRACKETED_PASTE_START.len()..].to_string();
            self.paste_mode = true;
            self.paste_buffer = std::mem::take(&mut self.buffer);
            if let Some(end_index) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let pasted_content = self.paste_buffer[..end_index].to_string();
                let remaining =
                    self.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                self.pending_kitty_printable_codepoint = None;
                events.paste.push(pasted_content);
                if !remaining.is_empty() {
                    events.extend(self.process(&remaining));
                }
            }
            return events;
        }

        let result = extract_complete_sequences(&self.buffer);
        self.buffer = result.remainder;
        for sequence in result.sequences {
            self.emit_data_sequence(&mut events, sequence);
        }
        if !self.buffer.is_empty() {
            let timeout_ms = if self.buffer == ESC {
                self.escape_timeout_ms
            } else {
                self.timeout_ms
            };
            self.timeout_at = Some(self.clock_ms.saturating_add(timeout_ms));
        }
        events
    }

    fn emit_data_sequence(&mut self, events: &mut StdinEvents, sequence: String) {
        let raw_codepoint = if sequence.chars().count() == 1 {
            sequence.chars().next().map(|ch| ch as u32)
        } else {
            None
        };
        if raw_codepoint.is_some() && raw_codepoint == self.pending_kitty_printable_codepoint {
            self.pending_kitty_printable_codepoint = None;
            return;
        }
        self.pending_kitty_printable_codepoint =
            parse_unmodified_kitty_printable_codepoint(&sequence);
        events.data.push(sequence);
    }
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::new()
    }
}

fn is_complete_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with(ESC) {
        return SequenceStatus::NotEscape;
    }
    if data.len() == 1 {
        return SequenceStatus::Incomplete;
    }
    let after_esc = &data[1..];
    if after_esc.starts_with('[') {
        if after_esc.starts_with("[M") {
            return if data.len() >= 6 {
                SequenceStatus::Complete
            } else {
                SequenceStatus::Incomplete
            };
        }
        return is_complete_csi_sequence(data);
    }
    if after_esc.starts_with(']') {
        return is_complete_osc_sequence(data);
    }
    if after_esc.starts_with('P') {
        return is_complete_dcs_sequence(data);
    }
    if after_esc.starts_with('_') {
        return is_complete_apc_sequence(data);
    }
    if after_esc.starts_with('O') {
        return if after_esc.len() >= 2 {
            SequenceStatus::Complete
        } else {
            SequenceStatus::Incomplete
        };
    }
    if after_esc.chars().count() == 1 {
        return SequenceStatus::Complete;
    }
    SequenceStatus::Complete
}

fn is_complete_csi_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b[") {
        return SequenceStatus::Complete;
    }
    if data.len() < 3 {
        return SequenceStatus::Incomplete;
    }
    let payload = &data[2..];
    let Some(last_char) = payload.chars().last() else {
        return SequenceStatus::Incomplete;
    };
    let last_char_code = last_char as u32;
    if (0x40..=0x7e).contains(&last_char_code) {
        if payload.starts_with('<') {
            if regex_mouse_sgr(payload) {
                return SequenceStatus::Complete;
            }
            if last_char == 'M' || last_char == 'm' {
                let inner = &payload[1..payload.len() - 1];
                let parts: Vec<&str> = inner.split(';').collect();
                if parts.len() == 3
                    && parts
                        .iter()
                        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                {
                    return SequenceStatus::Complete;
                }
            }
            return SequenceStatus::Incomplete;
        }
        return SequenceStatus::Complete;
    }
    SequenceStatus::Incomplete
}

fn regex_mouse_sgr(payload: &str) -> bool {
    let bytes = payload.as_bytes();
    if bytes.len() < 8 || bytes[0] != b'<' {
        return false;
    }
    let last = *bytes.last().unwrap();
    if last != b'M' && last != b'm' {
        return false;
    }
    let inner = &payload[1..payload.len() - 1];
    let parts: Vec<&str> = inner.split(';').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn is_complete_osc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b]") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") || data.ends_with('\u{07}') {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_dcs_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1bP") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_apc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b_") {
        return SequenceStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn parse_unmodified_kitty_printable_codepoint(sequence: &str) -> Option<u32> {
    let rest = sequence.strip_prefix("\x1b[")?.strip_suffix('u')?;
    if rest.contains(';') {
        return None;
    }
    let mut parts = rest.split(':');
    let codepoint = parts.next()?.parse::<u32>().ok()?;
    if let Some(second) = parts.next() {
        if !second.is_empty() && second.parse::<u32>().is_err() {
            return None;
        }
    }
    if parts
        .next()
        .is_some_and(|third| third.parse::<u32>().is_err())
    {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    if codepoint >= 32 {
        Some(codepoint)
    } else {
        None
    }
}

struct Extracted {
    sequences: Vec<String>,
    remainder: String,
}

fn extract_complete_sequences(buffer: &str) -> Extracted {
    let mut sequences = Vec::new();
    let mut pos = 0;
    while pos < buffer.len() {
        let remaining = &buffer[pos..];
        if remaining.starts_with(ESC) {
            let mut seq_end = 1;
            while seq_end <= remaining.len() {
                while seq_end < remaining.len() && !remaining.is_char_boundary(seq_end) {
                    seq_end += 1;
                }
                let candidate = &remaining[..seq_end];
                match is_complete_sequence(candidate) {
                    SequenceStatus::Complete => {
                        if candidate == "\x1b\x1b" {
                            let next_char = remaining[seq_end..].chars().next();
                            if matches!(next_char, Some('[' | ']' | 'O' | 'P' | '_')) {
                                sequences.push(ESC.to_string());
                                pos += 1;
                                break;
                            }
                        }
                        sequences.push(candidate.to_string());
                        pos += seq_end;
                        break;
                    }
                    SequenceStatus::Incomplete => {
                        seq_end += 1;
                    }
                    SequenceStatus::NotEscape => {
                        sequences.push(candidate.to_string());
                        pos += seq_end;
                        break;
                    }
                }
            }
            if seq_end > remaining.len() {
                return Extracted {
                    sequences,
                    remainder: remaining.to_string(),
                };
            }
        } else {
            let ch = remaining.chars().next().unwrap();
            sequences.push(ch.to_string());
            pos += ch.len_utf8();
        }
    }
    Extracted {
        sequences,
        remainder: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with_timeout(timeout: u64) -> StdinBuffer {
        StdinBuffer::with_options(StdinBufferOptions {
            timeout,
            escape_timeout: DEFAULT_ESCAPE_TIMEOUT_MS,
        })
    }

    #[test]
    fn passes_through_regular_and_unicode_characters() {
        let mut buffer = buffer_with_timeout(10);
        assert_eq!(buffer.process("a").data, ["a"]);
        assert_eq!(buffer.process("abc").data, ["a", "b", "c"]);
        assert_eq!(
            buffer.process("hello 世界").data,
            ["h", "e", "l", "l", "o", " ", "世", "界"]
        );
    }

    #[test]
    fn passes_through_complete_escape_sequences() {
        let mut buffer = buffer_with_timeout(10);
        assert_eq!(buffer.process("\x1b[<35;20;5m").data, ["\x1b[<35;20;5m"]);
        assert_eq!(buffer.process("\x1b[A").data, ["\x1b[A"]);
        assert_eq!(buffer.process("\x1b[11~").data, ["\x1b[11~"]);
        assert_eq!(buffer.process("\x1ba").data, ["\x1ba"]);
        assert_eq!(buffer.process("\x1bOA").data, ["\x1bOA"]);
    }

    #[test]
    fn buffers_incomplete_mouse_and_csi() {
        let mut buffer = buffer_with_timeout(10);
        assert!(buffer.process("\x1b").data.is_empty());
        assert_eq!(buffer.get_buffer(), "\x1b");
        assert!(buffer.process("[<35").data.is_empty());
        assert_eq!(buffer.get_buffer(), "\x1b[<35");
        assert_eq!(buffer.process(";20;5m").data, ["\x1b[<35;20;5m"]);
        assert_eq!(buffer.get_buffer(), "");

        assert!(buffer.process("\x1b[").data.is_empty());
        assert!(buffer.process("1;").data.is_empty());
        assert_eq!(buffer.process("5H").data, ["\x1b[1;5H"]);
    }

    #[test]
    fn buffers_split_across_many_chunks() {
        let mut buffer = buffer_with_timeout(10);
        for chunk in ["\x1b", "[", "<", "3", "5", ";", "2", "0", ";", "5", "m"] {
            let events = buffer.process(chunk);
            if chunk == "m" {
                assert_eq!(events.data, ["\x1b[<35;20;5m"]);
            } else {
                assert!(events.data.is_empty());
            }
        }
    }

    #[test]
    fn flush_and_timeout_paths_match_ts() {
        let mut buffer = buffer_with_timeout(10);
        assert!(buffer.process("\x1b[<35").data.is_empty());
        assert_eq!(buffer.tick(15).data, ["\x1b[<35"]);

        let mut buffer = buffer_with_timeout(10);
        assert!(buffer.process("\x1b").data.is_empty());
        assert_eq!(buffer.tick(20).data, ["\x1b"]);
        assert_eq!(buffer.process("\r").data, ["\r"]);

        let mut buffer = StdinBuffer::with_options(StdinBufferOptions {
            timeout: 50,
            escape_timeout: 100,
        });
        assert!(buffer.process("\x1b").data.is_empty());
        assert!(buffer.tick(20).data.is_empty());
        assert_eq!(buffer.process("\r").data, ["\x1b\r"]);

        let mut buffer = StdinBuffer::with_options(StdinBufferOptions {
            timeout: 100,
            escape_timeout: DEFAULT_ESCAPE_TIMEOUT_MS,
        });
        assert!(buffer.process("\x1b").data.is_empty());
        assert_eq!(buffer.tick(20).data, ["\x1b"]);
        assert_eq!(buffer.process("\r").data, ["\r"]);

        let mut delayed = StdinBuffer::new();
        assert!(delayed.process("\x1b[").data.is_empty());
        assert!(delayed.tick(20).data.is_empty());
        assert_eq!(delayed.process("<65;48;39M").data, ["\x1b[<65;48;39M"]);
    }

    #[test]
    fn mixed_content_and_kitty_protocol() {
        let mut buffer = buffer_with_timeout(10);
        assert_eq!(buffer.process("abc\x1b[A").data, ["a", "b", "c", "\x1b[A"]);
        assert_eq!(buffer.process("\x1b[Aabc").data, ["\x1b[A", "a", "b", "c"]);
        assert_eq!(
            buffer.process("\x1b[A\x1b[B\x1b[C").data,
            ["\x1b[A", "\x1b[B", "\x1b[C"]
        );
        assert_eq!(buffer.process("abc\x1b[<35").data, ["a", "b", "c"]);
        assert_eq!(buffer.get_buffer(), "\x1b[<35");
        assert_eq!(buffer.process(";20;5m").data, ["\x1b[<35;20;5m"]);

        assert_eq!(buffer.process("\x1b[97u").data, ["\x1b[97u"]);
        assert_eq!(buffer.process("\x1b[97;1:3u").data, ["\x1b[97;1:3u"]);
        assert_eq!(
            buffer.process("\x1b[97u\x1b[97;1:3u").data,
            ["\x1b[97u", "\x1b[97;1:3u"]
        );
        assert_eq!(
            buffer
                .process("\x1b[97u\x1b[97;1:3u\x1b[98u\x1b[98;1:3u")
                .data,
            ["\x1b[97u", "\x1b[97;1:3u", "\x1b[98u", "\x1b[98;1:3u"]
        );
        assert_eq!(buffer.process("\x1b[1;1:1A").data, ["\x1b[1;1:1A"]);
        assert_eq!(buffer.process("\x1b[3;1:3~").data, ["\x1b[3;1:3~"]);
        assert_eq!(
            buffer.process("\x1b\x1b[27;129:3u").data,
            ["\x1b", "\x1b[27;129:3u"]
        );
        assert_eq!(
            buffer.process("\x1b\x1b[27;1:3u").data,
            ["\x1b", "\x1b[27;1:3u"]
        );
        assert_eq!(buffer.process("\x1b\x1b").data, ["\x1b\x1b"]);
        assert_eq!(buffer.process("a\x1b[97;1:3u").data, ["a", "\x1b[97;1:3u"]);
        assert_eq!(buffer.process("\x1b[224uà").data, ["\x1b[224u"]);
        assert_eq!(buffer.process("\x1b[64u").data, ["\x1b[64u"]);
        assert!(buffer.process("@").data.is_empty());
        assert_eq!(buffer.process("\x1b[97ub").data, ["\x1b[97u", "b"]);
        assert_eq!(buffer.process("\x1b[64;3u@").data, ["\x1b[64;3u", "@"]);
        assert_eq!(
            buffer
                .process("\x1b[104u\x1b[104;1:3u\x1b[105u\x1b[105;1:3u")
                .data,
            ["\x1b[104u", "\x1b[104;1:3u", "\x1b[105u", "\x1b[105;1:3u"]
        );
    }

    #[test]
    fn mouse_old_style_and_edge_cases() {
        let mut buffer = buffer_with_timeout(10);
        assert_eq!(buffer.process("\x1b[<0;10;5M").data, ["\x1b[<0;10;5M"]);
        assert_eq!(buffer.process("\x1b[<0;10;5m").data, ["\x1b[<0;10;5m"]);
        assert_eq!(buffer.process("\x1b[<35;20;5m").data, ["\x1b[<35;20;5m"]);
        assert!(buffer.process("\x1b[<3").data.is_empty());
        assert!(buffer.process("5;1").data.is_empty());
        assert!(buffer.process("5;").data.is_empty());
        assert_eq!(buffer.process("10m").data, ["\x1b[<35;15;10m"]);
        assert_eq!(
            buffer
                .process("\x1b[<35;1;1m\x1b[<35;2;2m\x1b[<35;3;3m")
                .data,
            ["\x1b[<35;1;1m", "\x1b[<35;2;2m", "\x1b[<35;3;3m"]
        );
        assert_eq!(buffer.process("\x1b[M abc").data, ["\x1b[M ab", "c"]);
        assert!(buffer.process("\x1b[M").data.is_empty());
        assert_eq!(buffer.get_buffer(), "\x1b[M");
        assert!(buffer.process(" a").data.is_empty());
        assert_eq!(buffer.get_buffer(), "\x1b[M a");
        assert_eq!(buffer.process("b").data, ["\x1b[M ab"]);

        assert_eq!(buffer.process("").data, [""]);
        assert!(buffer.process("\x1b").data.is_empty());
        assert_eq!(buffer.tick(15).data, ["\x1b"]);
        let mut default_buffer = StdinBuffer::new();
        assert!(default_buffer.process("\x1b").data.is_empty());
        assert_eq!(default_buffer.tick(20).data, ["\x1b"]);
        assert!(buffer.process("\x1b").data.is_empty());
        assert_eq!(buffer.flush(), ["\x1b"]);
        assert_eq!(buffer.process_bytes(b"\x1b[A").data, ["\x1b[A"]);
        let long_seq = format!("\x1b[{}H", "1;".repeat(50));
        assert_eq!(buffer.process(&long_seq).data, [long_seq.as_str()]);
    }

    #[test]
    fn flush_clear_destroy_and_paste() {
        let mut buffer = buffer_with_timeout(10);
        assert!(buffer.process("\x1b[<35").data.is_empty());
        assert_eq!(buffer.flush(), ["\x1b[<35"]);
        assert_eq!(buffer.get_buffer(), "");
        assert!(buffer.flush().is_empty());
        assert!(buffer.process("\x1b[<35").data.is_empty());
        assert_eq!(buffer.tick(15).data, ["\x1b[<35"]);

        assert!(buffer.process("\x1b[<35").data.is_empty());
        buffer.clear();
        assert_eq!(buffer.get_buffer(), "");

        let events = buffer.process("\x1b[200~hello world\x1b[201~");
        assert_eq!(events.paste, ["hello world"]);
        assert!(events.data.is_empty());

        assert!(buffer.process("\x1b[200~").paste.is_empty());
        assert!(buffer.process("hello ").paste.is_empty());
        assert_eq!(buffer.process("world\x1b[201~").paste, ["hello world"]);

        let before = buffer.process("a");
        let paste = buffer.process("\x1b[200~pasted\x1b[201~");
        let after = buffer.process("b");
        assert_eq!(before.data, ["a"]);
        assert_eq!(after.data, ["b"]);
        assert_eq!(paste.paste, ["pasted"]);
        assert!(paste.data.is_empty());

        assert_eq!(
            buffer
                .process("\x1b[200~line1\nline2\nline3\x1b[201~")
                .paste,
            ["line1\nline2\nline3"]
        );
        assert_eq!(
            buffer.process("\x1b[200~Hello 世界 🎉\x1b[201~").paste,
            ["Hello 世界 🎉"]
        );

        assert!(buffer.process("\x1b[<35").data.is_empty());
        buffer.destroy();
        assert_eq!(buffer.get_buffer(), "");
        assert!(buffer.tick(15).data.is_empty());
    }
}

//! Bounded LSP JSON-RPC transport with Content-Length framing, cancellation, and tombstones.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Maximum allowed LSP message size (8 MiB).
pub const MAX_FRAME_SIZE: usize = 8 * 1024 * 1024;
pub const MAX_HEADER_SIZE: usize = 16 * 1024;
const MAX_BUFFER_SIZE: usize = MAX_FRAME_SIZE + MAX_HEADER_SIZE;
const MAX_TOMBSTONES: usize = 1024;

/// Maximum number of in-flight concurrent requests before applying backpressure.
pub const MAX_PENDING_REQUESTS: usize = 64;

/// Formats a JSON string as an LSP Content-Length framed byte message.
pub fn lsp_frame(json: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
    out.extend_from_slice(json.as_bytes());
    out
}

/// Errors that can occur when parsing LSP frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameParseError {
    MissingContentLength,
    InvalidContentLength,
    DuplicateContentLength,
    OversizedFrame(usize),
    OversizedHeader(usize),
    InvalidUtf8Header,
}

impl std::fmt::Display for FrameParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContentLength => write!(f, "Missing Content-Length header"),
            Self::InvalidContentLength => write!(f, "Invalid Content-Length header number"),
            Self::DuplicateContentLength => write!(f, "Duplicate Content-Length header"),
            Self::OversizedFrame(len) => {
                write!(
                    f,
                    "Oversized frame: {len} bytes exceeds max {MAX_FRAME_SIZE}"
                )
            }
            Self::OversizedHeader(len) => write!(
                f,
                "Oversized LSP header: {len} bytes exceeds max {MAX_HEADER_SIZE}"
            ),
            Self::InvalidUtf8Header => write!(f, "Invalid UTF-8 in frame header or body"),
        }
    }
}

impl std::error::Error for FrameParseError {}

/// Incremental parser for Content-Length framed LSP messages.
#[derive(Debug, Default)]
pub struct LspFrameParser {
    buffer: Vec<u8>,
    error: Option<FrameParseError>,
}

impl LspFrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends incoming chunks to the internal buffer.
    pub fn feed(&mut self, chunk: &[u8]) {
        if self.error.is_some() {
            return;
        }
        let size = self.buffer.len().saturating_add(chunk.len());
        if size > MAX_BUFFER_SIZE {
            self.buffer.clear();
            self.error = Some(FrameParseError::OversizedFrame(size));
            return;
        }
        self.buffer.extend_from_slice(chunk);
    }

    /// Attempts to extract the next frame, if complete.
    pub fn next_frame(&mut self) -> Result<Option<String>, FrameParseError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let sep = b"\r\n\r\n";
        let Some(pos) = self.buffer.windows(sep.len()).position(|w| w == sep) else {
            if self.buffer.len() > MAX_HEADER_SIZE {
                let error = FrameParseError::OversizedHeader(self.buffer.len());
                self.buffer.clear();
                self.error = Some(error.clone());
                return Err(error);
            }
            return Ok(None);
        };
        if pos + sep.len() > MAX_HEADER_SIZE {
            let error = FrameParseError::OversizedHeader(pos + sep.len());
            self.buffer.clear();
            self.error = Some(error.clone());
            return Err(error);
        }

        let header_bytes = &self.buffer[..pos];
        let header_str =
            std::str::from_utf8(header_bytes).map_err(|_| FrameParseError::InvalidUtf8Header)?;

        let mut content_length: Option<usize> = None;
        for line in header_str.split("\r\n") {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("Content-Length:") {
                if content_length.is_some() {
                    return Err(FrameParseError::DuplicateContentLength);
                }
                let parsed = val
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| FrameParseError::InvalidContentLength)?;
                content_length = Some(parsed);
            }
        }

        let Some(len) = content_length else {
            return Err(FrameParseError::MissingContentLength);
        };

        if len > MAX_FRAME_SIZE {
            return Err(FrameParseError::OversizedFrame(len));
        }

        let body_start = pos + sep.len();
        if self.buffer.len() < body_start + len {
            // Partial read, wait for more data
            return Ok(None);
        }

        let body_bytes = self.buffer[body_start..body_start + len].to_vec();
        self.buffer.drain(..body_start + len);

        let body_str =
            String::from_utf8(body_bytes).map_err(|_| FrameParseError::InvalidUtf8Header)?;
        Ok(Some(body_str))
    }
}

/// A pending outgoing request tracked in the request table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: u64,
    pub method: String,
    pub created_at: Instant,
    pub deadline: Duration,
}

/// Request tracker enforcing bounds, timeouts, cancellation, and tombstones.
#[derive(Debug)]
pub struct RequestTable {
    next_id: u64,
    pending: HashMap<u64, PendingRequest>,
    tombstones: HashSet<u64>,
    tombstone_order: VecDeque<u64>,
    late_response_floor: u64,
}

impl Default for RequestTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestTable {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            pending: HashMap::new(),
            tombstones: HashSet::new(),
            tombstone_order: VecDeque::new(),
            late_response_floor: 0,
        }
    }

    pub fn allocate_request(&mut self, method: &str, deadline: Duration) -> Result<u64, String> {
        if self.pending.len() >= MAX_PENDING_REQUESTS {
            return Err(format!(
                "Max pending requests backpressure reached ({MAX_PENDING_REQUESTS})"
            ));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("LSP request IDs exhausted")?;
        self.pending.insert(
            id,
            PendingRequest {
                id,
                method: method.to_string(),
                created_at: Instant::now(),
                deadline,
            },
        );
        Ok(id)
    }

    /// Cancel a request by ID: records a tombstone so late responses cannot be accepted.
    pub fn cancel_request(&mut self, id: u64) -> Option<PendingRequest> {
        if let Some(req) = self.pending.remove(&id) {
            self.record_tombstone(id);
            Some(req)
        } else {
            None
        }
    }

    /// Handle an incoming response by ID.
    pub fn handle_response(&mut self, id: u64) -> Result<Option<PendingRequest>, String> {
        if self.tombstones.contains(&id)
            || (id > 0 && id <= self.late_response_floor && !self.pending.contains_key(&id))
        {
            // Late reply to cancelled/timed out request safely dropped
            return Ok(None);
        }
        match self.pending.remove(&id) {
            Some(req) => {
                if req.created_at.elapsed() > req.deadline {
                    self.record_tombstone(id);
                    return Err(format!("Request {id} expired past deadline"));
                }
                Ok(Some(req))
            }
            None => Err(format!("Unexpected or unknown response id: {id}")),
        }
    }

    /// Reap expired requests and record tombstones.
    pub fn check_deadlines(&mut self) -> Vec<u64> {
        let mut expired = Vec::new();
        for (&id, req) in &self.pending {
            if req.created_at.elapsed() > req.deadline {
                expired.push(id);
            }
        }
        for &id in &expired {
            self.pending.remove(&id);
            self.record_tombstone(id);
        }
        expired
    }

    fn record_tombstone(&mut self, id: u64) {
        if self.tombstones.insert(id) {
            self.tombstone_order.push_back(id);
        }
        if self.tombstone_order.len() > MAX_TOMBSTONES {
            let oldest = self.tombstone_order.pop_front().unwrap();
            self.tombstones.remove(&oldest);
            // IDs never repeat. Old retired replies can be dropped using a
            // scalar horizon; pending requests below it still resolve normally.
            self.late_response_floor = self.late_response_floor.max(oldest);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// Detects unsolicited server-initiated mutation effects that must be denied in V1.
pub fn is_unsolicited_effect(method: &str) -> bool {
    method == "workspace/applyEdit"
        || method == "workspace/executeCommand"
        || method == "workspace/createFiles"
        || method == "workspace/deleteFiles"
        || method == "workspace/renameFiles"
}

/// Sanitizes server stderr lines for safe logging without secret or control code leaks.
pub fn sanitize_stderr_line(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control() || *c == '\t' || *c == '\n')
        .take(1024)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unterminated_header_is_bounded() {
        let mut parser = LspFrameParser::new();
        for _ in 0..3 {
            parser.feed(&[b'x'; 8192]);
        }
        assert!(parser.next_frame().is_err());
        assert!(parser.buffer.len() <= 16384);
    }

    #[test]
    fn oversized_feed_is_rejected_without_retaining_it() {
        let mut parser = LspFrameParser::new();
        parser.feed(&vec![b'x'; MAX_FRAME_SIZE + 32768]);
        assert!(parser.next_frame().is_err());
        assert!(parser.buffer.len() <= MAX_FRAME_SIZE + 16384);
    }

    #[test]
    fn cancellation_history_stays_bounded_and_old_replies_stay_rejected() {
        let mut table = RequestTable::new();
        let pending = table
            .allocate_request("still pending", Duration::from_secs(60))
            .unwrap();
        for _ in 0..10000 {
            let id = table
                .allocate_request("cancel", Duration::from_secs(5))
                .unwrap();
            table.cancel_request(id).unwrap();
        }
        assert!(table.tombstones.len() <= 1024);
        assert_eq!(table.handle_response(2).unwrap(), None);
        assert_eq!(table.handle_response(9999).unwrap(), None);
        assert_eq!(table.handle_response(pending).unwrap().unwrap().id, pending);
        assert!(table.handle_response(10002).is_err());
    }

    #[test]
    fn completed_oversized_header_is_rejected() {
        let mut parser = LspFrameParser::new();
        let frame = format!(
            "X-Extra: {}\r\nContent-Length: 2\r\n\r\n{{}}",
            "x".repeat(MAX_HEADER_SIZE)
        );
        parser.feed(frame.as_bytes());
        assert!(matches!(
            parser.next_frame(),
            Err(FrameParseError::OversizedHeader(_))
        ));
    }

    #[test]
    fn maximum_frame_parses_across_reads() {
        let mut parser = LspFrameParser::new();
        let body = "x".repeat(MAX_FRAME_SIZE);
        let frame = lsp_frame(&body);
        for chunk in frame.chunks(8192) {
            parser.feed(chunk);
            if frame.len() % 8192 != chunk.len() {
                assert_eq!(parser.next_frame().unwrap(), None);
            }
        }
        assert_eq!(parser.next_frame().unwrap(), Some(body));
    }

    #[test]
    fn f10_length_counts_bytes() {
        let payload = r#"{"x":"é"}"#;
        assert_eq!(
            lsp_frame(payload),
            b"Content-Length: 10\r\n\r\n{\"x\":\"\xc3\xa9\"}".to_vec()
        );
    }

    #[test]
    fn test_utf8_byte_length() {
        let payload = r#"{"text":"こんにちは"}"#;
        let frame = lsp_frame(payload);
        let prefix = format!("Content-Length: {}\r\n\r\n", payload.len());
        assert!(frame.starts_with(prefix.as_bytes()));
    }

    #[test]
    fn test_split_headers_and_body() {
        let mut parser = LspFrameParser::new();
        let frame = lsp_frame(r#"{"msg":"hello"}"#);

        // Feed half of the frame
        let split_at = 12;
        parser.feed(&frame[..split_at]);
        assert_eq!(parser.next_frame().unwrap(), None);

        // Feed rest of the frame
        parser.feed(&frame[split_at..]);
        let msg = parser.next_frame().unwrap().expect("should parse frame");
        assert_eq!(msg, r#"{"msg":"hello"}"#);
    }

    #[test]
    fn test_multiple_frames_per_read() {
        let mut parser = LspFrameParser::new();
        let mut combined = lsp_frame(r#"{"id":1}"#);
        combined.extend_from_slice(&lsp_frame(r#"{"id":2}"#));

        parser.feed(&combined);
        let f1 = parser.next_frame().unwrap().unwrap();
        let f2 = parser.next_frame().unwrap().unwrap();
        let f3 = parser.next_frame().unwrap();

        assert_eq!(f1, r#"{"id":1}"#);
        assert_eq!(f2, r#"{"id":2}"#);
        assert_eq!(f3, None);
    }

    #[test]
    fn test_malformed_length() {
        let mut parser = LspFrameParser::new();
        parser.feed(b"Content-Length: notanumber\r\n\r\n{}");
        assert_eq!(
            parser.next_frame().unwrap_err(),
            FrameParseError::InvalidContentLength
        );

        let mut parser2 = LspFrameParser::new();
        parser2.feed(b"Other-Header: 123\r\n\r\n{}");
        assert_eq!(
            parser2.next_frame().unwrap_err(),
            FrameParseError::MissingContentLength
        );
    }

    #[test]
    fn test_out_of_order_replies() {
        let mut table = RequestTable::new();
        let id1 = table
            .allocate_request("textDocument/definition", Duration::from_secs(5))
            .unwrap();
        let id2 = table
            .allocate_request("textDocument/references", Duration::from_secs(5))
            .unwrap();

        // Reply for id2 arrives before id1
        let req2 = table.handle_response(id2).unwrap().unwrap();
        assert_eq!(req2.id, id2);
        assert_eq!(req2.method, "textDocument/references");

        let req1 = table.handle_response(id1).unwrap().unwrap();
        assert_eq!(req1.id, id1);
        assert_eq!(req1.method, "textDocument/definition");
    }

    #[test]
    fn test_cancel_then_late_reply() {
        let mut table = RequestTable::new();
        let id = table
            .allocate_request("textDocument/outline", Duration::from_secs(5))
            .unwrap();

        // Cancel the request
        let cancelled = table.cancel_request(id);
        assert!(cancelled.is_some());

        // Late response arrives for cancelled request
        let res = table.handle_response(id).unwrap();
        assert_eq!(res, None, "Late response dropped because of tombstone");
    }

    #[test]
    fn test_stderr_flood_sanitization() {
        let raw = "LSP server warning: \x1b[31minvalid token\x1b[0m\x00\x07";
        let sanitized = sanitize_stderr_line(raw);
        assert!(!sanitized.contains('\x00'));
        assert!(!sanitized.contains('\x07'));
        assert!(sanitized.contains("LSP server warning:"));
    }

    #[test]
    fn test_request_deadline() {
        let mut table = RequestTable::new();
        let id = table
            .allocate_request("test/slow", Duration::from_millis(10))
            .unwrap();

        std::thread::sleep(Duration::from_millis(20));
        let expired = table.check_deadlines();
        assert_eq!(expired, vec![id]);

        // Attempting to handle response for expired request fails and records tombstone
        let res = table.handle_response(id).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn test_max_pending_backpressure() {
        let mut table = RequestTable::new();
        for _ in 0..MAX_PENDING_REQUESTS {
            table
                .allocate_request("method", Duration::from_secs(5))
                .unwrap();
        }
        assert_eq!(table.pending_count(), 64);

        let overflow = table.allocate_request("overflow", Duration::from_secs(5));
        assert!(overflow.is_err());
        assert!(overflow.unwrap_err().contains("backpressure"));
    }

    #[test]
    fn test_unsolicited_effect_detection() {
        assert!(is_unsolicited_effect("workspace/applyEdit"));
        assert!(is_unsolicited_effect("workspace/executeCommand"));
        assert!(!is_unsolicited_effect("textDocument/publishDiagnostics"));
        assert!(!is_unsolicited_effect("window/showMessage"));
    }
}

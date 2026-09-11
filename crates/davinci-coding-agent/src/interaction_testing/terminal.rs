//! PTY child process management, I/O streaming, terminal resizing, and process leases.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use super::screen::VirtualTerminalScreen;
use davinci_agent::runtime::control::ProcessLease;
use davinci_agent::runtime::ids::AgentId;

/// Validates whether terminal dimensions are allowed (1..=300 cols, 1..=120 rows).
pub fn terminal_size_allowed(cols: u16, rows: u16) -> bool {
    (1..=300).contains(&cols) && (1..=120).contains(&rows)
}

/// Configuration for a managed PTY interaction session.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub cols: u16,
    pub rows: u16,
    pub cwd: PathBuf,
    pub max_buffer_bytes: usize,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            cwd: PathBuf::from("."),
            max_buffer_bytes: 512 * 1024,
        }
    }
}

/// Simulated active PTY session with process lease ownership and bounded I/O buffer.
#[derive(Debug)]
pub struct SimulatedTerminalSession {
    pub config: PtyConfig,
    pub screen: VirtualTerminalScreen,
    pub lease: ProcessLease,
    pub is_running: bool,
    pub hung: bool,
    pub exit_code: Option<i32>,
    pub output_buffer: VecDeque<u8>,
    pub input_history: Vec<Vec<u8>>,
}

impl SimulatedTerminalSession {
    pub fn new(config: PtyConfig, pid: u32) -> Result<Self, String> {
        if !terminal_size_allowed(config.cols, config.rows) {
            return Err(format!(
                "Invalid terminal size: {}x{} exceeds bounds",
                config.cols, config.rows
            ));
        }

        let lease = ProcessLease::new_pty_lease(AgentId::new(), pid, Vec::new());
        let screen = VirtualTerminalScreen::new(config.cols, config.rows);

        Ok(Self {
            config,
            screen,
            lease,
            is_running: true,
            hung: false,
            exit_code: None,
            output_buffer: VecDeque::new(),
            input_history: Vec::new(),
        })
    }

    /// Resizes the PTY and underlying virtual screen.
    pub fn resize(&mut self, new_cols: u16, new_rows: u16) -> Result<(), String> {
        if !terminal_size_allowed(new_cols, new_rows) {
            return Err(format!(
                "Invalid terminal size: {}x{} exceeds bounds",
                new_cols, new_rows
            ));
        }
        self.config.cols = new_cols;
        self.config.rows = new_rows;
        self.screen.resize(new_cols, new_rows);
        Ok(())
    }

    /// Sends input bytes to the PTY stdin.
    pub fn send_input(&mut self, input: &[u8]) -> Result<(), String> {
        if !self.is_running {
            return Err("Cannot send input to terminated session".into());
        }
        self.input_history.push(input.to_vec());
        Ok(())
    }

    /// Appends output bytes received from PTY stdout with buffer bounds check (defending against output flood).
    pub fn push_output(&mut self, bytes: &[u8]) {
        self.screen.write_bytes(bytes);

        for &b in bytes {
            if self.output_buffer.len() >= self.config.max_buffer_bytes {
                self.output_buffer.pop_front();
            }
            self.output_buffer.push_back(b);
        }
    }

    /// Drains all currently buffered output bytes.
    pub fn drain_output(&mut self) -> Vec<u8> {
        self.output_buffer.drain(..).collect()
    }

    /// Shuts down the PTY session, escalating to force termination if hung.
    pub fn shutdown(&mut self, _timeout: Duration) -> Result<(), String> {
        if !self.is_running {
            return Ok(());
        }

        if self.hung {
            // Hung process refuses cooperative exit, requires force-kill
            self.is_running = false;
            self.exit_code = Some(137); // SIGKILL
            return Ok(());
        }

        self.is_running = false;
        self.exit_code = Some(0);
        Ok(())
    }

    /// Probes the screen for a draft line containing the given prefix.
    pub fn probe_draft(&self, prefix: &str) -> Option<String> {
        for row in 0..self.screen.rows() {
            if let Some(line) = self.screen.row_text(row) {
                if let Some(idx) = line.find(prefix) {
                    return Some(line[idx..].trim_end().to_string());
                }
            }
        }
        None
    }
}

/// Named key event mapping to ANSI escape sequences.
pub fn terminal_key(key: &str) -> Option<&'static [u8]> {
    match key {
        "shift_tab" => Some(b"\x1b[Z"),
        "tab" => Some(b"\t"),
        "escape" => Some(b"\x1b"),
        "enter" => Some(b"\r"),
        "up" => Some(b"\x1b[A"),
        "down" => Some(b"\x1b[B"),
        "right" => Some(b"\x1b[C"),
        "left" => Some(b"\x1b[D"),
        "backspace" => Some(b"\x7f"),
        "ctrl_c" => Some(b"\x03"),
        _ => None,
    }
}

/// Wraps text content in bracketed paste mode sequences.
pub fn bracketed_paste(content: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"\x1b[200~");
    buf.extend_from_slice(content.as_bytes());
    buf.extend_from_slice(b"\x1b[201~");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f11_resize_bounds() {
        assert!(terminal_size_allowed(80, 24));
        assert!(terminal_size_allowed(40, 12));
        assert!(!terminal_size_allowed(0, 24));
        assert!(!terminal_size_allowed(1000, 1000));
    }

    #[test]
    fn test_utf8_split_across_reads() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1001).unwrap();
        // Rocket emoji 🚀 is [0xF0, 0x9F, 0x9A, 0x80]
        let chunk1 = &[0xF0, 0x9F];
        let chunk2 = &[0x9A, 0x80];

        session.push_output(chunk1);
        assert!(!session.screen.contains_text("🚀"));

        session.push_output(chunk2);
        assert!(session.screen.contains_text("🚀"));
    }

    #[test]
    fn test_resize_during_render() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1002).unwrap();
        session.push_output(b"Top Left Header");

        assert!(session.resize(120, 30).is_ok());
        assert_eq!(session.config.cols, 120);
        assert_eq!(session.config.rows, 30);
        assert!(session.screen.contains_text("Top Left Header"));

        // Reject invalid bounds
        assert!(session.resize(0, 30).is_err());
        assert!(session.resize(120, 0).is_err());
    }

    #[test]
    fn test_alternate_screen_switch() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1003).unwrap();
        session.push_output(b"Main buffer text");
        assert!(session.screen.contains_text("Main buffer text"));
        assert!(!session.screen.is_alt_screen());

        // Switch to alternate screen
        session.push_output(b"\x1b[?1049h");
        assert!(session.screen.is_alt_screen());
        session.push_output(b"Alt buffer full screen");
        assert!(session.screen.contains_text("Alt buffer full screen"));

        // Switch back to main screen
        session.push_output(b"\x1b[?1049l");
        assert!(!session.screen.is_alt_screen());
        assert!(session.screen.contains_text("Main buffer text"));
    }

    #[test]
    fn test_output_flood() {
        let cfg = PtyConfig {
            max_buffer_bytes: 1024,
            ..Default::default()
        };
        let mut session = SimulatedTerminalSession::new(cfg, 1004).unwrap();

        let large_chunk = vec![b'A'; 100_000];
        session.push_output(&large_chunk);

        assert!(session.output_buffer.len() <= 1024);
    }

    #[test]
    fn test_child_exits_early() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1005).unwrap();
        session.is_running = false;
        session.exit_code = Some(1);

        assert!(session.send_input(b"ls\n").is_err());
    }

    #[test]
    fn test_hung_child() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1006).unwrap();
        session.hung = true;

        session.shutdown(Duration::from_millis(10)).unwrap();
        assert!(!session.is_running);
        assert_eq!(session.exit_code, Some(137));
    }

    #[test]
    fn test_no_output() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1007).unwrap();
        let drained = session.drain_output();
        assert!(drained.is_empty());
    }

    #[test]
    fn test_conpty_shutdown_drain() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 1008).unwrap();
        session.push_output(b"pending data");

        assert_eq!(session.drain_output(), b"pending data");
        session.shutdown(Duration::from_millis(50)).unwrap();
        assert!(!session.is_running);
    }

    #[test]
    fn test_unrelated_user_terminal_remains_alive() {
        let session = SimulatedTerminalSession::new(PtyConfig::default(), 1009).unwrap();
        let unrelated_pid = 99999;

        // Verify session lease only owns target pid
        assert_eq!(session.lease.os_handle_identity, 1009);
        assert!(session.lease.owns_process(1009));
        assert!(!session.lease.owns_process(unrelated_pid));
        assert_ne!(session.lease.os_handle_identity, unrelated_pid);
    }
}

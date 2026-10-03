//! Virtual terminal screen parser, ANSI escape sequence interpreter, and bounded frame capture.

use serde::{Deserialize, Serialize};

/// Captured snapshot frame of the terminal screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenFrame {
    pub cols: u16,
    pub rows: u16,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub alt_screen: bool,
    pub lines: Vec<String>,
}

/// In-memory virtual terminal screen maintaining primary and alternate screen grids.
#[derive(Debug, Clone)]
pub struct VirtualTerminalScreen {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    cursor_row: u16,
    alt_screen: bool,
    normal_grid: Vec<Vec<char>>,
    alt_grid: Vec<Vec<char>>,
    utf8_buf: Vec<u8>,
}

impl VirtualTerminalScreen {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let normal_grid = vec![vec![' '; cols as usize]; rows as usize];
        let alt_grid = vec![vec![' '; cols as usize]; rows as usize];

        Self {
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            alt_screen: false,
            normal_grid,
            alt_grid,
            utf8_buf: Vec::new(),
        }
    }

    fn grid_mut(&mut self) -> &mut Vec<Vec<char>> {
        if self.alt_screen {
            &mut self.alt_grid
        } else {
            &mut self.normal_grid
        }
    }

    fn grid(&self) -> &Vec<Vec<char>> {
        if self.alt_screen {
            &self.alt_grid
        } else {
            &self.normal_grid
        }
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn row_text(&self, row: u16) -> Option<String> {
        let r = row as usize;
        let g = self.grid();
        if r < g.len() {
            Some(g[r].iter().collect::<String>().trim_end().to_string())
        } else {
            None
        }
    }

    /// Resizes the virtual screen, preserving existing contents within new dimensions.
    pub fn resize(&mut self, new_cols: u16, new_rows: u16) {
        let new_cols = new_cols.max(1);
        let new_rows = new_rows.max(1);

        let resize_grid = |old_grid: &mut Vec<Vec<char>>| {
            old_grid.resize(new_rows as usize, vec![' '; new_cols as usize]);
            for row in old_grid.iter_mut() {
                row.resize(new_cols as usize, ' ');
            }
        };

        resize_grid(&mut self.normal_grid);
        resize_grid(&mut self.alt_grid);

        self.cols = new_cols;
        self.rows = new_rows;
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.utf8_buf.extend_from_slice(bytes);
        let mut parsed_text = String::new();
        let valid_up_to = match std::str::from_utf8(&self.utf8_buf) {
            Ok(s) => {
                parsed_text.push_str(s);
                self.utf8_buf.len()
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    if let Ok(s) = std::str::from_utf8(&self.utf8_buf[..valid]) {
                        parsed_text.push_str(s);
                    }
                }
                valid
            }
        };

        self.utf8_buf.drain(..valid_up_to);

        if !parsed_text.is_empty() {
            self.interpret_str(&parsed_text);
        }
    }

    fn interpret_str(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let mut idx = 0;

        while idx < chars.len() {
            let ch = chars[idx];

            if ch == '\x1b' && idx + 1 < chars.len() && chars[idx + 1] == '[' {
                // CSI sequence
                let mut end = idx + 2;
                while end < chars.len() && !(chars[end].is_ascii_alphabetic() || chars[end] == '~')
                {
                    end += 1;
                }
                if end < chars.len() {
                    let cmd_char = chars[end];
                    let params: String = chars[idx + 2..end].iter().collect();
                    self.execute_csi(&params, cmd_char);
                    idx = end + 1;
                    continue;
                }
            }

            match ch {
                '\r' => {
                    self.cursor_col = 0;
                }
                '\n' => {
                    if self.cursor_row + 1 < self.rows {
                        self.cursor_row += 1;
                    } else {
                        // Scroll up by 1 line
                        let cols = self.cols as usize;
                        let grid = self.grid_mut();
                        grid.remove(0);
                        grid.push(vec![' '; cols]);
                    }
                }
                '\x08' => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                '\t' => {
                    self.cursor_col = ((self.cursor_col / 8) + 1) * 8;
                    if self.cursor_col >= self.cols {
                        self.cursor_col = self.cols.saturating_sub(1);
                    }
                }
                c if !c.is_control() => {
                    let r = self.cursor_row as usize;
                    let col = self.cursor_col as usize;
                    if r < self.rows as usize && col < self.cols as usize {
                        self.grid_mut()[r][col] = c;
                    }
                    self.cursor_col += 1;
                    if self.cursor_col >= self.cols {
                        self.cursor_col = 0;
                        if self.cursor_row + 1 < self.rows {
                            self.cursor_row += 1;
                        }
                    }
                }
                _ => {}
            }
            idx += 1;
        }
    }

    fn execute_csi(&mut self, params: &str, cmd: char) {
        if params == "?1049" && cmd == 'h' {
            self.alt_screen = true;
            return;
        }
        if params == "?1049" && cmd == 'l' {
            self.alt_screen = false;
            return;
        }

        match cmd {
            'H' | 'f' => {
                // Cursor position
                let parts: Vec<&str> = params.split(';').collect();
                let row: u16 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(1);
                let col: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
                self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
            }
            'A' => {
                let count: u16 = params.parse().unwrap_or(1);
                self.cursor_row = self.cursor_row.saturating_sub(count);
            }
            'B' => {
                let count: u16 = params.parse().unwrap_or(1);
                self.cursor_row = (self.cursor_row + count).min(self.rows.saturating_sub(1));
            }
            'C' => {
                let count: u16 = params.parse().unwrap_or(1);
                self.cursor_col = (self.cursor_col + count).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let count: u16 = params.parse().unwrap_or(1);
                self.cursor_col = self.cursor_col.saturating_sub(count);
            }
            'J' => {
                // Erase in display
                if params == "2" {
                    let cols = self.cols as usize;
                    for row in self.grid_mut().iter_mut() {
                        *row = vec![' '; cols];
                    }
                    self.cursor_col = 0;
                    self.cursor_row = 0;
                }
            }
            'K' => {
                // Erase in line
                let r = self.cursor_row as usize;
                let c = self.cursor_col as usize;
                if r < self.rows as usize {
                    let cols = self.cols as usize;
                    for i in c..cols {
                        self.grid_mut()[r][i] = ' ';
                    }
                }
            }
            _ => {}
        }
    }

    /// Captures the current snapshot frame.
    pub fn capture_frame(&self) -> ScreenFrame {
        let lines: Vec<String> = self
            .grid()
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect();

        ScreenFrame {
            cols: self.cols,
            rows: self.rows,
            cursor_col: self.cursor_col,
            cursor_row: self.cursor_row,
            alt_screen: self.alt_screen,
            lines,
        }
    }

    /// Converts the entire screen into plain text.
    pub fn to_plain_text(&self) -> String {
        self.grid()
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Checks if a given substring exists anywhere on the active screen.
    pub fn contains_text(&self, pattern: &str) -> bool {
        self.to_plain_text().contains(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_text_writing_and_clear() {
        let mut screen = VirtualTerminalScreen::new(40, 10);
        screen.write_bytes(b"Hello Terminal!\r\nSecond line");
        assert!(screen.contains_text("Hello Terminal!"));
        assert!(screen.contains_text("Second line"));

        screen.write_bytes(b"\x1b[2J");
        assert_eq!(screen.to_plain_text().trim(), "");
    }
}

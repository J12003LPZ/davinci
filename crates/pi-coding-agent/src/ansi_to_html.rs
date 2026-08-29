//! TypeScript `export-html/ansi-to-html.ts` — ANSI SGR to HTML.

const ANSI_COLORS: [&str; 16] = [
    "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
    "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
];

#[derive(Clone, Default)]
struct TextStyle {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl TextStyle {
    fn to_inline_css(&self) -> String {
        let mut parts = Vec::new();
        if let Some(fg) = &self.fg {
            parts.push(format!("color:{fg}"));
        }
        if let Some(bg) = &self.bg {
            parts.push(format!("background-color:{bg}"));
        }
        if self.bold {
            parts.push("font-weight:bold".into());
        }
        if self.dim {
            parts.push("opacity:0.6".into());
        }
        if self.italic {
            parts.push("font-style:italic".into());
        }
        if self.underline {
            parts.push("text-decoration:underline".into());
        }
        parts.join(";")
    }

    fn has_style(&self) -> bool {
        self.fg.is_some()
            || self.bg.is_some()
            || self.bold
            || self.dim
            || self.italic
            || self.underline
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn color256_to_hex(index: u16) -> String {
    let index = index.min(255) as usize;
    if index < 16 {
        return ANSI_COLORS[index].to_string();
    }
    if index < 232 {
        let cube = index - 16;
        let r = cube / 36;
        let g = (cube % 36) / 6;
        let b = cube % 6;
        let to_hex = |n: usize| {
            let value = if n == 0 { 0 } else { 55 + n * 40 };
            format!("{value:02x}")
        };
        return format!("#{}{}{}", to_hex(r), to_hex(g), to_hex(b));
    }
    let gray = 8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            _ => out.push(ch),
        }
    }
    out
}

fn apply_sgr_code(params: &[u16], style: &mut TextStyle) {
    let mut i = 0;
    while i < params.len() {
        let code = params[i];
        if code == 0 {
            style.reset();
        } else if code == 1 {
            style.bold = true;
        } else if code == 2 {
            style.dim = true;
        } else if code == 3 {
            style.italic = true;
        } else if code == 4 {
            style.underline = true;
        } else if code == 22 {
            style.bold = false;
            style.dim = false;
        } else if code == 23 {
            style.italic = false;
        } else if code == 24 {
            style.underline = false;
        } else if (30..=37).contains(&code) {
            style.fg = Some(ANSI_COLORS[(code - 30) as usize].to_string());
        } else if code == 38 {
            if params.get(i + 1) == Some(&5) && params.len() > i + 2 {
                style.fg = Some(color256_to_hex(params[i + 2]));
                i += 2;
            } else if params.get(i + 1) == Some(&2) && params.len() > i + 4 {
                style.fg = Some(format!(
                    "rgb({},{},{})",
                    params[i + 2],
                    params[i + 3],
                    params[i + 4]
                ));
                i += 4;
            }
        } else if code == 39 {
            style.fg = None;
        } else if (40..=47).contains(&code) {
            style.bg = Some(ANSI_COLORS[(code - 40) as usize].to_string());
        } else if code == 48 {
            if params.get(i + 1) == Some(&5) && params.len() > i + 2 {
                style.bg = Some(color256_to_hex(params[i + 2]));
                i += 2;
            } else if params.get(i + 1) == Some(&2) && params.len() > i + 4 {
                style.bg = Some(format!(
                    "rgb({},{},{})",
                    params[i + 2],
                    params[i + 3],
                    params[i + 4]
                ));
                i += 4;
            }
        } else if code == 49 {
            style.bg = None;
        } else if (90..=97).contains(&code) {
            style.fg = Some(ANSI_COLORS[(code - 90 + 8) as usize].to_string());
        } else if (100..=107).contains(&code) {
            style.bg = Some(ANSI_COLORS[(code - 100 + 8) as usize].to_string());
        }
        i += 1;
    }
}

/// TypeScript `ansiToHtml`.
pub fn ansi_to_html(text: &str) -> String {
    let mut style = TextStyle::default();
    let mut result = String::new();
    let mut last_index = 0;
    let mut in_span = false;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some((end, params)) = parse_sgr(&text[i..]) {
                let before = &text[last_index..i];
                if !before.is_empty() {
                    result.push_str(&escape_html(before));
                }
                if in_span {
                    result.push_str("</span>");
                    in_span = false;
                }
                apply_sgr_code(&params, &mut style);
                if style.has_style() {
                    result.push_str(&format!("<span style=\"{}\">", style.to_inline_css()));
                    in_span = true;
                }
                last_index = i + end;
                i = last_index;
                continue;
            }
        }
        i += 1;
    }
    let remaining = &text[last_index..];
    if !remaining.is_empty() {
        result.push_str(&escape_html(remaining));
    }
    if in_span {
        result.push_str("</span>");
    }
    result
}

fn parse_sgr(text: &str) -> Option<(usize, Vec<u16>)> {
    let rest = text.strip_prefix("\u{1b}[")?;
    let end = rest.find('m')?;
    let param_str = &rest[..end];
    let params = if param_str.is_empty() {
        vec![0]
    } else {
        param_str
            .split(';')
            .map(|p| p.parse::<u16>().unwrap_or(0))
            .collect()
    };
    Some((2 + end + 1, params))
}

/// TypeScript `ansiLinesToHtml`.
pub fn ansi_lines_to_html(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| {
            let inner = ansi_to_html(line);
            let inner = if inner.is_empty() {
                "&nbsp;".to_string()
            } else {
                inner
            };
            format!("<div class=\"ansi-line\">{inner}</div>")
        })
        .collect::<Vec<_>>()
        .join("")
}

fn is_blank_rendered_line(line: &str) -> bool {
    strip_sgr(line).trim().is_empty()
}

fn strip_sgr(line: &str) -> String {
    let mut out = String::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some((end, _)) = parse_sgr(&line[i..]) {
                i += end;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// TypeScript `trimRenderedResultLines`.
pub fn trim_rendered_result_lines(lines: &[String]) -> Vec<String> {
    let mut start = 0;
    let mut end = lines.len();
    while start < end && is_blank_rendered_line(&lines[start]) {
        start += 1;
    }
    while end > start && is_blank_rendered_line(&lines[end - 1]) {
        end -= 1;
    }
    lines[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_lines_match_typescript_whitespace_fixture() {
        assert_eq!(
            ansi_lines_to_html(&["one".into(), "two".into()]),
            "<div class=\"ansi-line\">one</div><div class=\"ansi-line\">two</div>"
        );
        let lines = vec![
            String::new(),
            "\u{1b}[31mone\u{1b}[0m".into(),
            "two".into(),
            String::new(),
        ];
        let trimmed = trim_rendered_result_lines(&lines);
        assert_eq!(
            ansi_lines_to_html(&trimmed),
            "<div class=\"ansi-line\"><span style=\"color:#800000\">one</span></div><div class=\"ansi-line\">two</div>"
        );
    }
}

use std::collections::HashMap;

/// Symbol table matching `vendor/pi/packages/tui/src/latex.ts` `SYMBOLS`.
pub fn latex_symbols() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("alpha", "α"),
        ("beta", "β"),
        ("gamma", "γ"),
        ("delta", "δ"),
        ("pi", "π"),
        ("Pi", "Π"),
        ("sigma", "σ"),
        ("Sigma", "Σ"),
        ("omega", "ω"),
        ("Omega", "Ω"),
        ("times", "×"),
        ("div", "÷"),
        ("pm", "±"),
        ("cdot", "·"),
        ("infty", "∞"),
        ("leq", "≤"),
        ("geq", "≥"),
        ("neq", "≠"),
        ("rightarrow", "→"),
        ("leftarrow", "←"),
        ("sum", "∑"),
        ("prod", "∏"),
        ("int", "∫"),
        ("sqrt", "√"),
    ])
}

/// Render a basic LaTeX math expression as terminal Unicode.
/// Returns `None` when the expression contains unsupported syntax (TS contract).
pub fn render_latex(source: &str, _display: bool) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    if trimmed.contains("\\begin{") && !trimmed.contains("pmatrix") && !trimmed.contains("matrix") {
        return None;
    }
    let symbols = latex_symbols();
    let mut out = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut name = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() {
                    name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Some(symbol) = symbols.get(name.as_str()) {
                out.push_str(symbol);
            } else {
                return None;
            }
        } else if ch == '{' || ch == '}' {
            continue;
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ts_symbols_and_rejects_unknown() {
        assert_eq!(
            render_latex("\\alpha + \\pi", false).as_deref(),
            Some("α + π")
        );
        assert!(render_latex("\\begin{align}x\\end{align}", false).is_none());
    }
}

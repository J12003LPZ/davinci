use serde::Deserialize;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct Tsconfig {
    #[serde(default)]
    pub references: Vec<TsconfigReference>,
    #[serde(default, rename = "compilerOptions")]
    pub compiler_options: Option<TsCompilerOptions>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TsconfigReference {
    pub path: String,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct TsCompilerOptions {
    pub composite: Option<bool>,
    #[serde(rename = "tsBuildInfoFile")]
    pub ts_build_info_file: Option<String>,
    #[serde(rename = "outDir")]
    pub out_dir: Option<String>,
}

pub fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if ch == '/' && chars.peek() == Some(&'/') {
            // Line comment: skip until newline
            chars.next(); // consume '/'
            for line_ch in chars.by_ref() {
                if line_ch == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            // Block comment: skip until '*/'
            chars.next(); // consume '*'
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }

    out
}

/// Drops commas that directly precede `}` or `]` (outside strings), which
/// TypeScript accepts in `tsconfig.json` but strict JSON does not.
pub fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.get(index + 1) {
                    out.push(*escaped);
                    index += 1;
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if ch == ',' {
            let next = chars[index + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
        index += 1;
    }
    out
}

/// Parses JSON with comments and trailing commas, as TypeScript reads tsconfig.
pub fn parse_jsonc<T: serde::de::DeserializeOwned>(content: &str) -> serde_json::Result<T> {
    serde_json::from_str(&strip_trailing_commas(&strip_json_comments(content)))
}

pub fn parse_tsconfig(dir: &Path) -> Option<(PathBuf, Tsconfig)> {
    let candidates = ["tsconfig.json", "tsconfig.base.json", "tsconfig.build.json"];
    for candidate in candidates {
        let tsconfig_file = dir.join(candidate);
        if tsconfig_file.is_file() {
            if let Ok(content) = std::fs::read_to_string(&tsconfig_file) {
                if let Ok(parsed) = parse_jsonc::<Tsconfig>(&content) {
                    return Some((tsconfig_file, parsed));
                }
            }
        }
    }
    None
}

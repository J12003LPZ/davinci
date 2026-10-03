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

pub fn parse_tsconfig(dir: &Path) -> Option<(PathBuf, Tsconfig)> {
    let candidates = ["tsconfig.json", "tsconfig.base.json", "tsconfig.build.json"];
    for candidate in candidates {
        let tsconfig_file = dir.join(candidate);
        if tsconfig_file.is_file() {
            if let Ok(content) = std::fs::read_to_string(&tsconfig_file) {
                let stripped = strip_json_comments(&content);
                if let Ok(parsed) = serde_json::from_str::<Tsconfig>(&stripped) {
                    return Some((tsconfig_file, parsed));
                }
            }
        }
    }
    None
}

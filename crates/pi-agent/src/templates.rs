use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    pub path: PathBuf,
    pub body: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

pub fn discover_prompt_templates(roots: &[PathBuf]) -> Vec<PromptTemplate> {
    let mut templates = Vec::new();
    for root in roots {
        if root.is_file() {
            if let Some(template) = load_template(root) {
                templates.push(template);
            }
            continue;
        }
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).max_depth(3).into_iter().flatten() {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("md") | Some("txt") | Some("prompt")
            ) {
                if let Some(template) = load_template(path) {
                    templates.push(template);
                }
            }
        }
    }
    templates
}

fn load_template(path: &Path) -> Option<PromptTemplate> {
    let raw = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = parse_frontmatter(&raw);
    let description = frontmatter
        .get("description")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let first = body
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            if first.chars().count() > 60 {
                format!("{}...", first.chars().take(60).collect::<String>())
            } else {
                first.to_string()
            }
        });
    Some(PromptTemplate {
        name: path.file_stem()?.to_str()?.to_string(),
        path: path.to_path_buf(),
        body,
        description,
        argument_hint: frontmatter.get("argument-hint").cloned(),
    })
}

/// TS `stripFrontmatter` / `parseFrontmatter` body extraction.
pub fn strip_frontmatter(content: &str) -> String {
    parse_frontmatter(content).1
}

pub fn parse_frontmatter(content: &str) -> (std::collections::BTreeMap<String, String>, String) {
    let normalized = content
        .strip_prefix('\u{feff}')
        .unwrap_or(content)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if !normalized.starts_with("---") {
        return (std::collections::BTreeMap::new(), normalized);
    }
    let Some(end) = normalized[3..].find("\n---") else {
        return (std::collections::BTreeMap::new(), normalized);
    };
    let yaml = &normalized[3..3 + end];
    let body = normalized[3 + end + 4..].trim().to_string();
    let mut fields = std::collections::BTreeMap::new();
    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        fields.insert(key.trim().to_string(), value);
    }
    (fields, body)
}

/// TS `parseCommandArgs` — bash-style quoted tokens.
pub fn parse_command_args(args_string: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for char in args_string.chars() {
        if let Some(quote) = in_quote {
            if char == quote {
                in_quote = None;
            } else {
                current.push(char);
            }
        } else if char == '"' || char == '\'' {
            in_quote = Some(char);
        } else if char.is_whitespace() {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(char);
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn substitute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)")
            .expect("substitute args regex")
    })
}

/// TS `substituteArgs`.
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all_args = args.join(" ");
    substitute_re()
        .replace_all(content, |caps: &regex::Captures| {
            if let Some(default_target) = caps.get(1) {
                let default_value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let value =
                    if default_target.as_str() == "@" || default_target.as_str() == "ARGUMENTS" {
                        all_args.as_str()
                    } else {
                        args.get(
                            default_target
                                .as_str()
                                .parse::<usize>()
                                .unwrap_or(0)
                                .saturating_sub(1),
                        )
                        .map(String::as_str)
                        .unwrap_or("")
                    };
                return if !value.is_empty() {
                    value.to_string()
                } else {
                    default_value.to_string()
                };
            }
            if let Some(slice_start) = caps.get(3) {
                let mut start = slice_start
                    .as_str()
                    .parse::<isize>()
                    .unwrap_or(1)
                    .saturating_sub(1);
                if start < 0 {
                    start = 0;
                }
                let start = start as usize;
                if let Some(slice_length) = caps.get(4) {
                    let length = slice_length.as_str().parse::<usize>().unwrap_or(0);
                    return args
                        .get(start..start.saturating_add(length).min(args.len()))
                        .unwrap_or(&[])
                        .join(" ");
                }
                return args.get(start..).unwrap_or(&[]).join(" ");
            }
            if let Some(simple) = caps.get(5) {
                if simple.as_str() == "ARGUMENTS" || simple.as_str() == "@" {
                    return all_args.clone();
                }
                let index = simple.as_str().parse::<isize>().unwrap_or(0) - 1;
                if index < 0 {
                    return String::new();
                }
                return args.get(index as usize).cloned().unwrap_or_default();
            }
            String::new()
        })
        .into_owned()
}

/// TS `expandPromptTemplate`.
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> String {
    if !text.starts_with('/') {
        return text.to_string();
    }
    let rest = &text[1..];
    let (name, args_string) = match rest.find(|c: char| c.is_whitespace()) {
        Some(index) => (&rest[..index], rest[index + 1..].to_string()),
        None => (rest, String::new()),
    };
    let Some(template) = templates.iter().find(|item| item.name == name) else {
        return text.to_string();
    };
    let args = parse_command_args(&args_string);
    substitute_args(&template.body, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn substitute_args_matches_ts_fixtures() {
        assert_eq!(
            substitute_args("Test: $ARGUMENTS", &args(&["a", "b", "c"])),
            "Test: a b c"
        );
        assert_eq!(
            substitute_args("Test: $@", &args(&["a", "b", "c"])),
            "Test: a b c"
        );
        assert_eq!(
            substitute_args("$ARGUMENTS", &args(&["$1", "$ARGUMENTS"])),
            "$1 $ARGUMENTS"
        );
        assert_eq!(
            substitute_args("$1: $ARGUMENTS", &args(&["prefix", "a", "b"])),
            "prefix: prefix a b"
        );
        assert_eq!(substitute_args("Test: $ARGUMENTS", &[]), "Test: ");
        assert_eq!(substitute_args("Test: $1", &[]), "Test: ");
        assert_eq!(
            substitute_args("$ARGUMENTS and $ARGUMENTS", &args(&["a", "b"])),
            "a b and a b"
        );
        assert_eq!(
            substitute_args("$1 $2: $ARGUMENTS", &args(&["arg100", "@user"])),
            "arg100 @user: arg100 @user"
        );
        assert_eq!(
            substitute_args("$1 $2 $3 $4 $5", &args(&["a", "b"])),
            "a b   "
        );
        assert_eq!(
            substitute_args("$ARGUMENTS", &args(&["日本語", "🎉", "café"])),
            "日本語 🎉 café"
        );
        assert_eq!(substitute_args("$1$2", &args(&["a", "b"])), "ab");
        assert_eq!(substitute_args("$0", &args(&["a", "b"])), "");
        assert_eq!(substitute_args("$1.5", &args(&["a"])), "a.5");
        assert_eq!(
            substitute_args("pre$ARGUMENTS", &args(&["a", "b"])),
            "prea b"
        );
        assert_eq!(
            substitute_args("$arguments $Arguments $ARGUMENTS", &args(&["a", "b"])),
            "$arguments $Arguments a b"
        );
        assert_eq!(substitute_args("Price: \\$100", &[]), "Price: \\");
        assert_eq!(
            substitute_args("$1: $@ ($ARGUMENTS)", &args(&["first", "second", "third"])),
            "first: first second third (first second third)"
        );
    }

    #[test]
    fn substitute_args_positional_defaults() {
        assert_eq!(
            substitute_args("List exactly ${1:-7} next steps", &[]),
            "List exactly 7 next steps"
        );
        assert_eq!(
            substitute_args("List exactly ${1:-7} next steps", &args(&["3"])),
            "List exactly 3 next steps"
        );
        assert_eq!(
            substitute_args("Mode: ${1:-brief}", &args(&[""])),
            "Mode: brief"
        );
        assert_eq!(substitute_args("${1:-7} ${2:-brief}", &[]), "7 brief");
        assert_eq!(
            substitute_args("${1:-7} ${2:-brief}", &args(&["3"])),
            "3 brief"
        );
        assert_eq!(
            substitute_args("${1:-7} ${2:-brief}", &args(&["3", "verbose"])),
            "3 verbose"
        );
        assert_eq!(
            substitute_args("${1:-7}", &args(&["$ARGUMENTS"])),
            "$ARGUMENTS"
        );
        assert_eq!(substitute_args("${1:-$ARGUMENTS}", &args(&["a", "b"])), "a");
        assert_eq!(
            substitute_args("${3:-$ARGUMENTS}", &args(&["a", "b"])),
            "$ARGUMENTS"
        );
        assert_eq!(
            substitute_args("$1 ${2:-x} $ARGUMENTS", &args(&["a"])),
            "a x a"
        );
    }

    #[test]
    fn substitute_args_array_slicing() {
        assert_eq!(
            substitute_args("${@:2}", &args(&["a", "b", "c", "d"])),
            "b c d"
        );
        assert_eq!(substitute_args("${@:1}", &args(&["a", "b", "c"])), "a b c");
        assert_eq!(
            substitute_args("${@:2:2}", &args(&["a", "b", "c", "d"])),
            "b c"
        );
        assert_eq!(substitute_args("${@:99}", &args(&["a", "b"])), "");
        assert_eq!(substitute_args("${@:2:0}", &args(&["a", "b", "c"])), "");
        assert_eq!(substitute_args("${@:2:99}", &args(&["a", "b", "c"])), "b c");
        assert_eq!(substitute_args("${@:0}", &args(&["a", "b", "c"])), "a b c");
        assert_eq!(substitute_args("${@:2}", &[]), "");
        assert_eq!(
            substitute_args("prefix${@:2}suffix", &args(&["a", "b", "c"])),
            "prefixb csuffix"
        );
        assert_eq!(
            substitute_args("${@:2}", &args(&["cmd", "$100", "@user", "#tag"])),
            "$100 @user #tag"
        );
    }

    #[test]
    fn parse_command_args_matches_ts() {
        assert_eq!(parse_command_args("a b c"), args(&["a", "b", "c"]));
        assert_eq!(
            parse_command_args("\"first arg\" second"),
            args(&["first arg", "second"])
        );
        assert_eq!(
            parse_command_args("'first arg' second"),
            args(&["first arg", "second"])
        );
        assert_eq!(parse_command_args(""), Vec::<String>::new());
        assert_eq!(parse_command_args("a  b   c"), args(&["a", "b", "c"]));
        assert_eq!(parse_command_args("a\tb\tc"), args(&["a", "b", "c"]));
        assert_eq!(parse_command_args("\"\" \" \""), args(&[" "]));
        assert_eq!(
            parse_command_args("$100 @user #tag"),
            args(&["$100", "@user", "#tag"])
        );
        assert_eq!(
            parse_command_args("日本語 🎉 café"),
            args(&["日本語", "🎉", "café"])
        );
        assert_eq!(
            parse_command_args("\"line1\nline2\" second"),
            args(&["line1\nline2", "second"])
        );
        assert_eq!(
            parse_command_args("label-2\n\nHere is some description #2."),
            args(&["label-2", "Here", "is", "some", "description", "#2."])
        );
        assert_eq!(parse_command_args("a\n\n\tb  c"), args(&["a", "b", "c"]));
        assert_eq!(
            parse_command_args("\"quoted \\\"text\\\"\""),
            args(&["quoted \\text\\"])
        );
    }

    #[test]
    fn expand_prompt_template_splits_on_newlines() {
        let templates = vec![PromptTemplate {
            name: "arg-test".into(),
            path: PathBuf::from("/tmp/arg-test.md"),
            body: "- arg1: $1\n- rest: ${@:2}".into(),
            description: "test".into(),
            argument_hint: None,
        }];
        assert_eq!(
            expand_prompt_template(
                "/arg-test label-2\n\nHere is some description #2.",
                &templates
            ),
            "- arg1: label-2\n- rest: Here is some description #2."
        );
        assert_eq!(
            expand_prompt_template(
                "/arg-test\nlabel-2",
                &[PromptTemplate {
                    name: "arg-test".into(),
                    path: PathBuf::from("/tmp/arg-test.md"),
                    body: "arg1: $1".into(),
                    description: "test".into(),
                    argument_hint: None,
                }]
            ),
            "arg1: label-2"
        );
        let review = vec![PromptTemplate {
            name: "review".into(),
            path: PathBuf::from("/virtual/review.md"),
            body: "Review this code: $1".into(),
            description: "Review template".into(),
            argument_hint: None,
        }];
        assert_eq!(
            expand_prompt_template("/review src/index.ts", &review),
            "Review this code: src/index.ts"
        );
        assert_eq!(expand_prompt_template("plain", &review), "plain");
        assert_eq!(
            expand_prompt_template("/missing args", &review),
            "/missing args"
        );
    }

    #[test]
    fn strip_frontmatter_matches_ts() {
        assert_eq!(strip_frontmatter("---\nname: x\n---\n\nBody"), "Body");
        assert_eq!(
            strip_frontmatter("\n  No frontmatter body  \n"),
            "\n  No frontmatter body  \n"
        );
    }
}

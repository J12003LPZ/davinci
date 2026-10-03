//! A one-screen lexer for the code the transcript shows: keywords, strings,
//! comments and numbers in their own ink, everything else in the caller's.
//!
//! Each line is lexed alone — a hunk row, a fenced-code row — so a block
//! comment that spans lines colours only its first and last; the cost is
//! taken for a lexer that needs no state and no dependency. Colour comes
//! from `Theme::syntax`, never from here (design.md §2).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/highlight.ex`.

use std::path::Path;

/// What a run of a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Plain,
}

/// The languages the lexer knows. Anything else is plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    C,
    Cpp,
    Java,
    Kotlin,
    CSharp,
    Ruby,
    Php,
    Swift,
    Shell,
    PowerShell,
    Json,
    Toml,
    Yaml,
    Sql,
    Elixir,
    Html,
    Css,
}

/// The language of a path (`src/lib.rs`) or a fence tag (`rust`, `py`).
pub fn language_of(name: &str) -> Option<Lang> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| name.to_ascii_lowercase());
    Some(match ext.as_str() {
        "rs" | "rust" => Lang::Rust,
        "js" | "jsx" | "mjs" | "cjs" | "javascript" => Lang::JavaScript,
        "ts" | "tsx" | "mts" | "cts" | "typescript" => Lang::TypeScript,
        "py" | "pyi" | "python" => Lang::Python,
        "go" | "golang" => Lang::Go,
        "c" | "h" => Lang::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" => Lang::Cpp,
        "java" => Lang::Java,
        "kt" | "kts" | "kotlin" => Lang::Kotlin,
        "cs" | "csharp" | "c#" => Lang::CSharp,
        "rb" | "ruby" | "rake" | "gemspec" => Lang::Ruby,
        "php" => Lang::Php,
        "swift" => Lang::Swift,
        "sh" | "bash" | "zsh" | "shell" | "fish" | "console" => Lang::Shell,
        "ps1" | "psm1" | "powershell" | "pwsh" => Lang::PowerShell,
        "json" | "jsonc" | "json5" => Lang::Json,
        "toml" => Lang::Toml,
        "yml" | "yaml" => Lang::Yaml,
        "sql" | "psql" | "mysql" => Lang::Sql,
        "ex" | "exs" | "elixir" => Lang::Elixir,
        "html" | "htm" | "xml" | "svg" | "vue" | "svelte" => Lang::Html,
        "css" | "scss" | "less" => Lang::Css,
        _ => return None,
    })
}

struct Grammar {
    keywords: &'static [&'static str],
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    /// Keywords match regardless of case (SQL).
    case_insensitive: bool,
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "union",
];
const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "of",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "static",
    "string",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "self", "match", "case",
];
const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "nil",
    "true",
    "false",
    "error",
    "string",
    "int",
    "bool",
    "byte",
];
const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "NULL", "true", "false", "bool", "size_t",
    "uint8_t", "uint32_t", "uint64_t", "int32_t", "int64_t", "define", "include", "ifdef",
    "ifndef", "endif", "pragma",
];
const CPP_KEYWORDS: &[&str] = &[
    "alignas",
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "constexpr",
    "const_cast",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
    "include",
    "define",
    "pragma",
    "ifdef",
    "ifndef",
    "endif",
    "std",
    "size_t",
];
const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "false",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];
const KOTLIN_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "const",
    "constructor",
    "continue",
    "data",
    "do",
    "else",
    "enum",
    "false",
    "final",
    "finally",
    "for",
    "fun",
    "if",
    "import",
    "in",
    "init",
    "interface",
    "internal",
    "is",
    "lateinit",
    "null",
    "object",
    "open",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "sealed",
    "super",
    "suspend",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "where",
    "while",
];
const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "false",
    "finally",
    "float",
    "for",
    "foreach",
    "get",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "set",
    "short",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "where",
    "while",
    "yield",
];
const RUBY_KEYWORDS: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "require",
    "attr_accessor",
    "attr_reader",
    "private",
    "puts",
    "raise",
];
const PHP_KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "if",
    "implements",
    "include",
    "instanceof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "null",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "true",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "yield",
];
const SWIFT_KEYWORDS: &[&str] = &[
    "as",
    "associatedtype",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "Self",
    "some",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
    "any",
];
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "return", "export", "local", "readonly", "set", "unset", "shift", "exit",
    "source", "alias", "select", "time", "true", "false", "declare", "eval", "exec",
];
const POWERSHELL_KEYWORDS: &[&str] = &[
    "begin",
    "break",
    "catch",
    "class",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "enum",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "from",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "while",
    "workflow",
    "true",
    "false",
    "null",
];
const JSON_KEYWORDS: &[&str] = &["true", "false", "null"];
const TOML_KEYWORDS: &[&str] = &["true", "false", "inf", "nan"];
const YAML_KEYWORDS: &[&str] = &[
    "true", "false", "null", "yes", "no", "on", "off", "True", "False", "Null", "Yes", "No",
    "TRUE", "FALSE", "NULL",
];
const SQL_KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "drop",
    "alter",
    "add",
    "column",
    "index",
    "join",
    "inner",
    "left",
    "right",
    "outer",
    "full",
    "on",
    "as",
    "and",
    "or",
    "not",
    "null",
    "is",
    "in",
    "exists",
    "between",
    "like",
    "order",
    "by",
    "group",
    "having",
    "limit",
    "offset",
    "union",
    "all",
    "distinct",
    "primary",
    "key",
    "foreign",
    "references",
    "unique",
    "default",
    "constraint",
    "begin",
    "commit",
    "rollback",
    "transaction",
    "with",
    "case",
    "when",
    "then",
    "else",
    "end",
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "true",
    "false",
    "integer",
    "text",
    "varchar",
    "boolean",
    "timestamp",
    "returning",
    "cascade",
    "if",
    "view",
    "explain",
    "analyze",
];
const ELIXIR_KEYWORDS: &[&str] = &[
    "def",
    "defp",
    "defmodule",
    "defmacro",
    "defstruct",
    "defimpl",
    "defprotocol",
    "do",
    "end",
    "fn",
    "if",
    "else",
    "unless",
    "case",
    "cond",
    "when",
    "with",
    "for",
    "receive",
    "after",
    "raise",
    "rescue",
    "try",
    "catch",
    "throw",
    "import",
    "alias",
    "require",
    "use",
    "true",
    "false",
    "nil",
    "and",
    "or",
    "not",
    "in",
    "quote",
    "unquote",
    "@moduledoc",
    "@doc",
    "@spec",
    "@impl",
];
const CSS_KEYWORDS: &[&str] = &[
    "important",
    "media",
    "import",
    "keyframes",
    "font-face",
    "supports",
    "inherit",
    "initial",
    "unset",
    "none",
    "auto",
    "block",
    "flex",
    "grid",
    "inline",
    "absolute",
    "relative",
    "fixed",
    "sticky",
    "hidden",
    "visible",
    "solid",
    "bold",
    "normal",
];

fn grammar(lang: Lang) -> Grammar {
    let c_like = |keywords| Grammar {
        keywords,
        line_comment: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"', '\''],
        case_insensitive: false,
    };
    let hash = |keywords| Grammar {
        keywords,
        line_comment: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        case_insensitive: false,
    };
    match lang {
        Lang::Rust => Grammar {
            keywords: RUST_KEYWORDS,
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            case_insensitive: false,
        },
        Lang::JavaScript => Grammar {
            keywords: JS_KEYWORDS,
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            case_insensitive: false,
        },
        Lang::TypeScript => Grammar {
            keywords: TS_KEYWORDS,
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            case_insensitive: false,
        },
        Lang::Python => hash(PYTHON_KEYWORDS),
        Lang::Go => Grammar {
            keywords: GO_KEYWORDS,
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            case_insensitive: false,
        },
        Lang::C => c_like(C_KEYWORDS),
        Lang::Cpp => c_like(CPP_KEYWORDS),
        Lang::Java => c_like(JAVA_KEYWORDS),
        Lang::Kotlin => c_like(KOTLIN_KEYWORDS),
        Lang::CSharp => c_like(CSHARP_KEYWORDS),
        Lang::Ruby => hash(RUBY_KEYWORDS),
        Lang::Php => Grammar {
            keywords: PHP_KEYWORDS,
            line_comment: &["//", "#"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            case_insensitive: true,
        },
        Lang::Swift => c_like(SWIFT_KEYWORDS),
        Lang::Shell => Grammar {
            keywords: SHELL_KEYWORDS,
            line_comment: &["#"],
            block_comment: None,
            quotes: &['"', '\'', '`'],
            case_insensitive: false,
        },
        Lang::PowerShell => Grammar {
            keywords: POWERSHELL_KEYWORDS,
            line_comment: &["#"],
            block_comment: Some(("<#", "#>")),
            quotes: &['"', '\''],
            case_insensitive: true,
        },
        Lang::Json => Grammar {
            keywords: JSON_KEYWORDS,
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            case_insensitive: false,
        },
        Lang::Toml => hash(TOML_KEYWORDS),
        Lang::Yaml => hash(YAML_KEYWORDS),
        Lang::Sql => Grammar {
            keywords: SQL_KEYWORDS,
            line_comment: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: &['\'', '"'],
            case_insensitive: true,
        },
        Lang::Elixir => hash(ELIXIR_KEYWORDS),
        Lang::Html => Grammar {
            keywords: &[],
            line_comment: &[],
            block_comment: Some(("<!--", "-->")),
            quotes: &['"', '\''],
            case_insensitive: false,
        },
        Lang::Css => Grammar {
            keywords: CSS_KEYWORDS,
            line_comment: &[],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            case_insensitive: true,
        },
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == '@'
}

fn is_ident(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!' || ch == '-'
}

/// Lex one line into `(token, text)` runs; adjacent plain runs are merged,
/// and the runs concatenate back to the line.
pub fn tokens(lang: Option<Lang>, line: &str) -> Vec<(Token, String)> {
    let Some(lang) = lang else {
        return vec![(Token::Plain, line.to_string())];
    };
    if line.is_empty() {
        return Vec::new();
    }
    let g = grammar(lang);
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(Token, String)> = Vec::new();
    let mut push = |token: Token, text: String| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((last, run)) if *last == token => run.push_str(&text),
            _ => out.push((token, text)),
        }
    };
    let mut i = 0usize;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        // Comments to the end of the line.
        if g.line_comment.iter().any(|prefix| rest.starts_with(prefix)) {
            push(Token::Comment, rest);
            break;
        }
        // Block comments, closed on this line or not.
        if let Some((open, close)) = g.block_comment {
            if let Some(after) = rest.strip_prefix(open) {
                let end = after
                    .find(close)
                    .map(|at| open.len() + at + close.len())
                    .unwrap_or(rest.len());
                let taken: String = rest[..end].to_string();
                i += taken.chars().count();
                push(Token::Comment, taken);
                continue;
            }
        }
        let ch = chars[i];
        // Strings, with escapes; an unclosed string runs to the end.
        if g.quotes.contains(&ch) {
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == ch {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(chars.len());
            push(Token::String, chars[i..j].iter().collect());
            i = j;
            continue;
        }
        // HTML: the tag name after `<` or `</`.
        if lang == Lang::Html && ch == '<' {
            let mut j = i + 1;
            if j < chars.len() && chars[j] == '/' {
                j += 1;
            }
            let name_start = j;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == ':')
            {
                j += 1;
            }
            if j > name_start {
                push(Token::Keyword, chars[i..j].iter().collect());
                i = j;
                continue;
            }
        }
        // Numbers: `42`, `0xff`, `1_000`, `3.14`, `10u8`.
        let after_ident = i > 0 && is_ident(chars[i - 1]) && chars[i - 1] != '-';
        if ch.is_ascii_digit() && !after_ident {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_ascii_alphanumeric() || chars[j] == '_' || chars[j] == '.')
            {
                j += 1;
            }
            push(Token::Number, chars[i..j].iter().collect());
            i = j;
            continue;
        }
        // Words: keywords or plain identifiers.
        if is_ident_start(ch) {
            let mut j = i + 1;
            while j < chars.len() && is_ident(chars[j]) {
                j += 1;
            }
            // A trailing `-` belongs to the next thing (`a-b` in YAML keys
            // is one word, `x-1` in code is not); keep it simple: words end
            // before a `-` that is followed by a digit.
            let word: String = chars[i..j].iter().collect();
            let is_keyword = if g.case_insensitive {
                let lower = word.to_ascii_lowercase();
                g.keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&lower))
            } else {
                g.keywords.contains(&word.as_str())
            };
            push(
                if is_keyword {
                    Token::Keyword
                } else {
                    Token::Plain
                },
                word,
            );
            i = j;
            continue;
        }
        push(Token::Plain, ch.to_string());
        i += 1;
    }
    out
}

/// One line as spans: each run in the ink `Theme::syntax` gives its token,
/// plain runs in `base`.
pub fn spans(
    theme: &crate::davinci::theme::Theme,
    lang: Option<Lang>,
    line: &str,
    base: ratatui::style::Color,
) -> Vec<ratatui::text::Span<'static>> {
    tokens(lang, line)
        .into_iter()
        .map(|(token, text)| crate::davinci::ui::span(text, theme.syntax(token, base)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Lang, line: &str) -> Vec<(Token, String)> {
        tokens(Some(lang), line)
    }

    fn joined(lang: Lang, line: &str) -> String {
        kinds(lang, line)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    #[test]
    fn languages_come_from_paths_and_fence_tags() {
        assert_eq!(
            language_of("crates/davinci-agent/src/lib.rs"),
            Some(Lang::Rust)
        );
        assert_eq!(language_of("rust"), Some(Lang::Rust));
        assert_eq!(language_of("app.tsx"), Some(Lang::TypeScript));
        assert_eq!(language_of("py"), Some(Lang::Python));
        assert_eq!(language_of("Makefile"), None);
        assert_eq!(language_of(""), None);
        assert_eq!(language_of("notes.md"), None);
        assert_eq!(language_of("run.ex"), Some(Lang::Elixir));
    }

    #[test]
    fn rust_keywords_strings_comments_and_numbers_are_told_apart() {
        let runs = kinds(Lang::Rust, "let x: u32 = 0xff; // hex \"quoted\"");
        assert_eq!(runs[0], (Token::Keyword, "let".into()));
        assert!(runs.iter().any(|(t, s)| *t == Token::Number && s == "0xff"));
        let comment = runs.last().unwrap();
        assert_eq!(comment.0, Token::Comment);
        assert_eq!(comment.1, "// hex \"quoted\"");
        assert_eq!(
            joined(Lang::Rust, "let x: u32 = 0xff; // hex \"quoted\""),
            "let x: u32 = 0xff; // hex \"quoted\""
        );

        let string = kinds(Lang::Rust, r#"println!("a \"b\" c", 1);"#);
        assert!(string
            .iter()
            .any(|(t, s)| *t == Token::String && s == r#""a \"b\" c""#));
        assert!(string.iter().any(|(t, s)| *t == Token::Number && s == "1"));
        // An identifier that ends in digits is not a number.
        let ident = kinds(Lang::Rust, "let u8x = a1;");
        assert!(!ident.iter().any(|(t, _)| *t == Token::Number), "{ident:?}");
    }

    #[test]
    fn each_family_knows_its_own_comment_and_string_forms() {
        assert_eq!(
            kinds(Lang::Python, "# note").last().unwrap().0,
            Token::Comment
        );
        assert_eq!(
            kinds(Lang::Python, "def f(): pass")[0],
            (Token::Keyword, "def".into())
        );
        assert_eq!(
            kinds(Lang::Sql, "SELECT id FROM t -- why")[0],
            (Token::Keyword, "SELECT".into())
        );
        assert_eq!(
            kinds(Lang::Sql, "select 1 -- why").last().unwrap().0,
            Token::Comment
        );
        let block = kinds(Lang::C, "int x; /* mid */ int y;");
        assert!(block
            .iter()
            .any(|(t, s)| *t == Token::Comment && s == "/* mid */"));
        let open = kinds(Lang::C, "/* still going");
        assert_eq!(open, vec![(Token::Comment, "/* still going".into())]);
        let js = kinds(Lang::JavaScript, "const s = `tpl ${x}`;");
        assert!(js
            .iter()
            .any(|(t, s)| *t == Token::String && s == "`tpl ${x}`"));
        let sh = kinds(Lang::Shell, "if [ -f x ]; then echo 'hi'; fi # done");
        assert_eq!(sh[0], (Token::Keyword, "if".into()));
        assert!(sh.iter().any(|(t, s)| *t == Token::String && s == "'hi'"));
        assert_eq!(sh.last().unwrap().0, Token::Comment);
        let ps = kinds(Lang::PowerShell, "ForEach ($f in $files) { <# c #> }");
        assert!(ps
            .iter()
            .any(|(t, s)| *t == Token::Keyword && s == "ForEach"));
        assert!(ps
            .iter()
            .any(|(t, s)| *t == Token::Comment && s == "<# c #>"));
        let html = kinds(Lang::Html, r#"<a href="x">hi</a> <!-- c -->"#);
        assert_eq!(html[0], (Token::Keyword, "<a".into()));
        assert!(html.iter().any(|(t, s)| *t == Token::Keyword && s == "</a"));
        assert!(html
            .iter()
            .any(|(t, s)| *t == Token::Comment && s == "<!-- c -->"));
        let toml = kinds(Lang::Toml, "enabled = true # yes");
        assert!(toml
            .iter()
            .any(|(t, s)| *t == Token::Keyword && s == "true"));
        let yaml = kinds(Lang::Yaml, "key-name: 12");
        assert_eq!(yaml[0], (Token::Plain, "key-name: ".into()));
        assert!(yaml.iter().any(|(t, s)| *t == Token::Number && s == "12"));
        let ex = kinds(Lang::Elixir, "defp entry({:tool, x}, th), do: 1");
        assert_eq!(ex[0], (Token::Keyword, "defp".into()));
    }

    #[test]
    fn punctuation_only_lines_and_unknown_languages_are_plain() {
        assert_eq!(
            kinds(Lang::Rust, "}) ;"),
            vec![(Token::Plain, "}) ;".into())]
        );
        assert_eq!(
            tokens(None, "anything at all"),
            vec![(Token::Plain, "anything at all".into())]
        );
        assert!(tokens(Some(Lang::Rust), "").is_empty());
        assert_eq!(
            joined(Lang::Go, "fmt.Println(\"x\", 3.14)"),
            "fmt.Println(\"x\", 3.14)"
        );
    }
}

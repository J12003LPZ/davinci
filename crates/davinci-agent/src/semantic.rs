//! Host-neutral semantic service trait, DTOs, and fallback mechanisms matching F10.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Contract helper: route semantic queries based on availability, support, and freshness.
pub fn semantic_route(server_available: bool, supported: bool, stale: bool) -> &'static str {
    if !server_available {
        "text_fallback"
    } else if !supported {
        "unsupported"
    } else if stale {
        "stale_requery"
    } else {
        "semantic"
    }
}

/// A zero-indexed position in a text document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// A range in a text document expressed as start and end positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Represents a location inside a resource, such as a line inside a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default)]
    pub is_textual_fallback: bool,
}

/// The severity of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// A diagnostic message, such as a compiler error or linter warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
}

/// Represents a programming construct like a variable, function, or class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub range: Range,
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Represents a call hierarchy item (function, method, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub range: Range,
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A call hierarchy incoming or outgoing call edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallHierarchyCall {
    pub item: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// Capabilities advertised by a language server for a given language/file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SemanticCapabilities {
    pub definition: bool,
    pub references: bool,
    pub outline: bool,
    pub diagnostics: bool,
    pub call_hierarchy: bool,
    pub rename_preview: bool,
}

/// A text replacement edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// A safe, validated preview of a workspace-wide rename operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenamePreview {
    pub id: String,
    pub server_identity: String,
    pub document_versions: HashMap<String, i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<String>,
    pub edits: HashMap<String, Vec<TextEdit>>,
    pub affected_paths: Vec<String>,
    pub digest: String,
}

/// Unified result from a semantic query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticResult {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_identity: Option<String>,
    pub capability: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<String>,
    #[serde(default)]
    pub locations: Vec<Location>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub symbols: Vec<SymbolItem>,
    #[serde(default)]
    pub calls: Vec<CallHierarchyCall>,
    #[serde(default)]
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticRequestContext {
    pub deadline: Option<std::time::Instant>,
    pub abort: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[derive(Debug, Clone)]
pub enum SemanticQuery<'a> {
    Definition {
        cwd: &'a Path,
        path: &'a str,
        position: Position,
    },
    References {
        cwd: &'a Path,
        path: &'a str,
        position: Position,
        include_declaration: bool,
    },
    Outline {
        cwd: &'a Path,
        path: &'a str,
    },
    Diagnostics {
        cwd: &'a Path,
        path: &'a str,
    },
}

/// Core interface for semantic capabilities, injected into ToolContext.
pub trait SemanticService: std::fmt::Debug + Send + Sync {
    fn is_server_available(&self, language: &str, path: &Path) -> bool;
    fn capabilities(&self, language: &str, path: &Path) -> SemanticCapabilities;
    fn definition(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String>;
    fn references(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String>;
    fn outline(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String>;
    fn diagnostics(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String>;
    fn call_hierarchy(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        incoming: bool,
    ) -> Result<SemanticResult, String>;
    fn rename_preview(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<RenamePreview, String>;

    fn query_with_context(
        &self,
        query: SemanticQuery<'_>,
        _context: &SemanticRequestContext,
    ) -> Result<SemanticResult, String> {
        match query {
            SemanticQuery::Definition {
                cwd,
                path,
                position,
            } => self.definition(cwd, path, position.line, position.character),
            SemanticQuery::References {
                cwd,
                path,
                position,
                include_declaration,
            } => self.references(
                cwd,
                path,
                position.line,
                position.character,
                include_declaration,
            ),
            SemanticQuery::Outline { cwd, path } => self.outline(cwd, path),
            SemanticQuery::Diagnostics { cwd, path } => self.diagnostics(cwd, path),
        }
    }
}

/// Largest source file the textual fallbacks read. A bigger file is scanned
/// only up to its last whole line inside this prefix, and the result says so.
const FALLBACK_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Most symbols the fallback outline reports.
const OUTLINE_MAX_SYMBOLS: usize = 1000;
/// Most definition hits the fallback reports.
const DEFINITION_MAX_HITS: usize = 50;
/// Most reference hits the fallback reports.
const REFERENCE_MAX_HITS: usize = 100;
/// Most code files a fallback search examines.
const FALLBACK_MAX_FILES: usize = 200;
/// Most directory entries a fallback search walks.
const FALLBACK_MAX_ENTRIES: usize = 50_000;

const INCOMPLETE_FILE_PREFIX: &str = "a file over 4 MiB was scanned only in part";
const INCOMPLETE_UNREADABLE: &str = "an unreadable file was skipped";
const INCOMPLETE_LINKED: &str = "a hard-linked file was skipped";
const INCOMPLETE_UNSAFE_ENTRY: &str =
    "symlinked, junction or out-of-workspace entries were not followed";
const INCOMPLETE_FILES: &str = "file discovery stopped at 200 files";
const INCOMPLETE_ENTRIES: &str = "directory walk stopped at 50000 entries";
const INCOMPLETE_DEFINITIONS: &str = "more than 50 definitions exist; only 50 are listed";
const INCOMPLETE_REFERENCES: &str = "more than 100 references exist; only 100 are listed";
const INCOMPLETE_SYMBOLS: &str = "more than 1000 symbols exist; only 1000 are listed";

/// Why a fallback result may be missing something. A result with any reason is
/// `partial`, never presented as a complete scan.
#[derive(Default)]
struct Incomplete(Vec<&'static str>);

impl Incomplete {
    fn note(&mut self, reason: &'static str) {
        if !self.0.contains(&reason) {
            self.0.push(reason);
        }
    }

    fn is_partial(&self) -> bool {
        !self.0.is_empty()
    }

    /// `base`, then the reasons the result is incomplete.
    fn reason(&self, base: &str) -> String {
        if self.0.is_empty() {
            base.to_string()
        } else {
            format!("{base}; incomplete: {}", self.0.join("; "))
        }
    }
}

/// A source file read for a fallback: its text, and whether it was cut to the
/// byte limit (at a line end, so the text never ends inside a character).
struct Source {
    text: String,
    prefix_only: bool,
}

/// The workspace as it was when a fallback began: its resolved path, read from
/// an opened directory handle, and that handle kept open for the whole search
/// (on Windows an open directory cannot be renamed or replaced, so the name
/// stays the same directory). Every source is judged against this path, never
/// against a later resolution of the workspace name, so a workspace renamed
/// and replaced by a link to elsewhere mid-search admits nothing new.
struct AdmittedRoot {
    path: PathBuf,
    _pin: fs::File,
}

impl AdmittedRoot {
    fn open(root: &Path) -> Result<Self, String> {
        let pin = davinci_sys::fs::open_directory_pinned(root)
            .map_err(|error| format!("cannot open the workspace: {error}"))?;
        let path = davinci_sys::fs::final_path(&pin)
            .map_err(|error| format!("cannot resolve the workspace: {error}"))?;
        Ok(Self { path, _pin: pin })
    }
}

#[cfg(test)]
type ReadHook = Box<dyn Fn(&Path)>;

#[cfg(test)]
thread_local! {
    /// Runs before each fallback source is opened, so a test can change the
    /// workspace after discovery.
    static SEMANTIC_BEFORE_READ: std::cell::RefCell<Option<ReadHook>> =
        const { std::cell::RefCell::new(None) };
}

/// Read `path` on its opened handle, admitting only a regular, singly linked
/// file that resolves beneath `root`, and reading at most
/// `FALLBACK_MAX_FILE_BYTES + 1` bytes of it. The confinement is judged on the
/// handle that supplies the bytes, so a link or junction put in place of the
/// file or one of its directories after discovery cannot redirect the read.
fn read_source(
    root: &AdmittedRoot,
    path: &Path,
    allow_linked: bool,
) -> Result<Source, &'static str> {
    use std::io::Read;
    #[cfg(test)]
    SEMANTIC_BEFORE_READ.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(path);
        }
    });
    let file = fs::File::open(path).map_err(|_| INCOMPLETE_UNREADABLE)?;
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return Err(INCOMPLETE_UNREADABLE);
    }
    match davinci_sys::fs::final_path(&file) {
        Ok(real) if real.starts_with(&root.path) => {}
        Ok(_) => return Err(INCOMPLETE_UNSAFE_ENTRY),
        // Where the platform cannot name the object, do not read it.
        Err(_) => return Err(INCOMPLETE_UNSAFE_ENTRY),
    }
    if !allow_linked {
        davinci_sys::fs::ensure_single_link(&file).map_err(|error| {
            if davinci_sys::fs::is_hard_link_refusal(&error) {
                INCOMPLETE_LINKED
            } else {
                INCOMPLETE_UNREADABLE
            }
        })?;
    }
    let mut bytes = Vec::new();
    file.take(FALLBACK_MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INCOMPLETE_UNREADABLE)?;
    let prefix_only = bytes.len() as u64 > FALLBACK_MAX_FILE_BYTES;
    if prefix_only {
        // Keep whole lines only: the cut can fall inside a UTF-8 sequence, and
        // a half line would be scanned as if it were the whole line.
        bytes.truncate(FALLBACK_MAX_FILE_BYTES as usize);
        let whole = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end + 1);
        bytes.truncate(whole);
    }
    let text = String::from_utf8(bytes).map_err(|_| INCOMPLETE_UNREADABLE)?;
    Ok(Source { text, prefix_only })
}

/// Characters that continue an identifier, as the definition scan judges them.
fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '$')
}

/// The protocol column of byte offset `byte` in `line`: protocol positions
/// count UTF-16 code units, not bytes.
fn utf16_column(line: &str, byte: usize) -> u32 {
    // Callers pass character boundaries; a bad offset must still not panic.
    let mut end = byte.min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    u32::try_from(line[..end].encode_utf16().count()).unwrap_or(u32::MAX)
}

/// The range of `line[start..end]` on zero-based line `line_idx`.
fn line_range(line_idx: usize, line: &str, start: usize, end: usize) -> Range {
    let line_idx = u32::try_from(line_idx).unwrap_or(u32::MAX);
    Range {
        start: Position {
            line: line_idx,
            character: utf16_column(line, start),
        },
        end: Position {
            line: line_idx,
            character: utf16_column(line, end),
        },
    }
}

fn is_aborted(aborted: Option<&std::sync::atomic::AtomicBool>) -> bool {
    aborted.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
}

/// The files a fallback search covers, and what kept the list incomplete.
fn fallback_search_files(
    cwd: &Path,
    target_path: Option<&Path>,
    incomplete: &mut Incomplete,
) -> Result<Vec<PathBuf>, String> {
    if let Some(tp) = target_path {
        let full = safe_workspace_path(cwd, tp)?;
        if full.is_file() {
            return Ok(vec![full]);
        }
        Ok(collect_code_files(
            &full,
            cwd,
            FALLBACK_MAX_FILES,
            incomplete,
        ))
    } else {
        Ok(collect_code_files(cwd, cwd, FALLBACK_MAX_FILES, incomplete))
    }
}

/// Fallback definition finder using text search patterns.
pub fn text_fallback_definition(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
) -> Result<SemanticResult, String> {
    text_fallback_definition_authorized(cwd, symbol, target_path, aborted, false)
}

/// [`text_fallback_definition`] for a caller whose permission policy already
/// authorizes reads outside the workspace (`allow_linked`), so a hard-linked
/// file is scanned instead of reported as skipped.
pub fn text_fallback_definition_authorized(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
    allow_linked: bool,
) -> Result<SemanticResult, String> {
    if is_aborted(aborted) {
        return Err("Operation aborted".into());
    }
    if symbol.trim().is_empty() {
        return Err("Symbol cannot be empty for definition lookup".into());
    }

    let mut locations = Vec::new();
    let mut incomplete = Incomplete::default();
    let root = AdmittedRoot::open(cwd)?;
    let def_keywords = [
        "fn ",
        "function ",
        "def ",
        "class ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "type ",
        "const ",
        "let ",
        "var ",
    ];

    let search_files = fallback_search_files(cwd, target_path, &mut incomplete)?;

    'files: for file in search_files {
        if is_aborted(aborted) {
            return Err("Operation aborted".into());
        }
        let source = match read_source(&root, &file, allow_linked) {
            Ok(source) => source,
            Err(reason) => {
                incomplete.note(reason);
                continue;
            }
        };
        if source.prefix_only {
            incomplete.note(INCOMPLETE_FILE_PREFIX);
        }
        for (line_idx, line) in source.text.lines().enumerate() {
            let trimmed = line.trim();
            // Offsets are taken from slices that keep the line's own end, so
            // `rest` is always a suffix of `line` (trailing whitespace would
            // otherwise shift every column).
            let leading = line.trim_start();
            let stripped = leading
                .strip_prefix("pub(crate) ")
                .or_else(|| leading.strip_prefix("pub "))
                .or_else(|| leading.strip_prefix("export default "))
                .or_else(|| leading.strip_prefix("export "))
                .or_else(|| leading.strip_prefix("async "))
                .unwrap_or(leading);
            let stripped_async = stripped.strip_prefix("async ").unwrap_or(stripped);
            let declared_symbol = def_keywords.iter().find_map(|keyword| {
                let rest = stripped_async.strip_prefix(keyword)?.trim_start();
                let suffix = rest.strip_prefix(symbol)?;
                (!suffix.chars().next().is_some_and(is_identifier_char))
                    .then_some(line.len().saturating_sub(rest.len()))
            });

            if let Some(col) = declared_symbol {
                if locations.len() >= DEFINITION_MAX_HITS {
                    incomplete.note(INCOMPLETE_DEFINITIONS);
                    break 'files;
                }
                let rel_path = file
                    .strip_prefix(cwd)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/");
                locations.push(Location {
                    path: rel_path,
                    range: line_range(line_idx, line, col, col + symbol.len()),
                    snippet: Some(trimmed.to_string()),
                    is_textual_fallback: true,
                });
            }
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_def_{symbol}"),
        server_identity: None,
        capability: "text_fallback_definition".into(),
        document_version: None,
        source_manifest: None,
        locations,
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: incomplete.is_partial(),
        fallback_reason: Some(incomplete.reason(
            "Language server unavailable or unsupported; results retrieved via text-pattern fallback",
        )),
    })
}

/// Fallback reference finder using word-based text search.
pub fn text_fallback_references(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
) -> Result<SemanticResult, String> {
    text_fallback_references_authorized(cwd, symbol, target_path, aborted, false)
}

/// [`text_fallback_references`] for an outside-read authorized caller.
pub fn text_fallback_references_authorized(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
    allow_linked: bool,
) -> Result<SemanticResult, String> {
    if is_aborted(aborted) {
        return Err("Operation aborted".into());
    }
    if symbol.trim().is_empty() {
        return Err("Symbol cannot be empty for references lookup".into());
    }

    let mut locations = Vec::new();
    let mut incomplete = Incomplete::default();
    let root = AdmittedRoot::open(cwd)?;
    let search_files = fallback_search_files(cwd, target_path, &mut incomplete)?;
    // A symbol that starts or ends with an identifier character must not sit
    // inside a longer identifier (`foo` in `foobar`, `_foo` or `foo2`).
    let guard_before = symbol.chars().next().is_some_and(is_identifier_char);
    let guard_after = symbol.chars().next_back().is_some_and(is_identifier_char);

    'files: for file in search_files {
        if is_aborted(aborted) {
            return Err("Operation aborted".into());
        }
        let source = match read_source(&root, &file, allow_linked) {
            Ok(source) => source,
            Err(reason) => {
                incomplete.note(reason);
                continue;
            }
        };
        if source.prefix_only {
            incomplete.note(INCOMPLETE_FILE_PREFIX);
        }
        for (line_idx, line) in source.text.lines().enumerate() {
            let mut from = 0;
            while let Some(found) = line[from..].find(symbol) {
                let start = from + found;
                let end = start + symbol.len();
                let before_ok = !guard_before
                    || !line[..start]
                        .chars()
                        .next_back()
                        .is_some_and(is_identifier_char);
                let after_ok =
                    !guard_after || !line[end..].chars().next().is_some_and(is_identifier_char);
                if !(before_ok && after_ok) {
                    // Step one character, not one byte, past a rejected hit.
                    from = start + line[start..].chars().next().map_or(1, char::len_utf8);
                    continue;
                }
                if locations.len() >= REFERENCE_MAX_HITS {
                    incomplete.note(INCOMPLETE_REFERENCES);
                    break 'files;
                }
                let rel_path = file
                    .strip_prefix(cwd)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/");
                locations.push(Location {
                    path: rel_path,
                    range: line_range(line_idx, line, start, end),
                    snippet: Some(line.trim().to_string()),
                    is_textual_fallback: true,
                });
                from = end;
            }
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_ref_{symbol}"),
        server_identity: None,
        capability: "text_fallback_references".into(),
        document_version: None,
        source_manifest: None,
        locations,
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: incomplete.is_partial(),
        fallback_reason: Some(incomplete.reason(
            "Language server unavailable or unsupported; references retrieved via text search",
        )),
    })
}

/// Fallback outline extracting top-level symbols via syntax patterns.
pub fn text_fallback_outline(cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
    text_fallback_outline_authorized(cwd, file_path, false)
}

/// [`text_fallback_outline`] for an outside-read authorized caller.
pub fn text_fallback_outline_authorized(
    cwd: &Path,
    file_path: &str,
    allow_linked: bool,
) -> Result<SemanticResult, String> {
    let root = AdmittedRoot::open(cwd)?;
    let full = safe_workspace_path(cwd, Path::new(file_path))?;
    let source = read_source(&root, &full, allow_linked)
        .map_err(|reason| format!("Could not read file {file_path}: {reason}"))?;
    let mut incomplete = Incomplete::default();
    if source.prefix_only {
        incomplete.note(INCOMPLETE_FILE_PREFIX);
    }

    let mut symbols = Vec::new();
    let def_keywords = [
        ("fn ", "function"),
        ("pub fn ", "function"),
        ("async fn ", "function"),
        ("pub async fn ", "function"),
        ("function ", "function"),
        ("def ", "function"),
        ("class ", "class"),
        ("struct ", "struct"),
        ("pub struct ", "struct"),
        ("enum ", "enum"),
        ("pub enum ", "enum"),
        ("trait ", "interface"),
        ("pub trait ", "interface"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("const ", "constant"),
    ];

    for (line_idx, line) in source.text.lines().enumerate() {
        let trimmed = line.trim_start();
        for (kw, kind) in &def_keywords {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                let rest = rest.trim_start();
                let name_len = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                if name_len > 0 {
                    if symbols.len() >= OUTLINE_MAX_SYMBOLS {
                        incomplete.note(INCOMPLETE_SYMBOLS);
                        break;
                    }
                    // `rest` is a suffix of `line`, so the identifier starts
                    // where the suffix does.
                    let name_start = line.len() - rest.len();
                    symbols.push(SymbolItem {
                        name: rest[..name_len].to_string(),
                        kind: (*kind).to_string(),
                        range: line_range(line_idx, line, 0, line.len()),
                        selection_range: line_range(
                            line_idx,
                            line,
                            name_start,
                            name_start + name_len,
                        ),
                        detail: Some(line.trim().to_string()),
                    });
                }
                break;
            }
        }
        if incomplete.0.contains(&INCOMPLETE_SYMBOLS) {
            break;
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_outline_{file_path}"),
        server_identity: None,
        capability: "text_fallback_outline".into(),
        document_version: None,
        source_manifest: None,
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols,
        calls: Vec::new(),
        partial: incomplete.is_partial(),
        fallback_reason: Some(incomplete.reason(
            "Language server unavailable or unsupported; outline generated from syntax patterns",
        )),
    })
}

/// Fallback diagnostic probe that preserves the explicit limitation that no
/// compiler or language server was run. Reading the file proves the target is
/// available, but an empty diagnostic set must remain partial rather than being
/// presented as proof that the file is error-free.
pub fn text_fallback_diagnostics(cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
    let full = safe_workspace_path(cwd, Path::new(file_path))?;
    fs::read_to_string(&full).map_err(|e| format!("Could not read file {file_path}: {e}"))?;

    Ok(SemanticResult {
        request_id: format!("fallback_diagnostics_{file_path}"),
        server_identity: None,
        capability: "text_fallback_diagnostics".into(),
        document_version: None,
        source_manifest: None,
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: true,
        fallback_reason: Some(
            "Language server unavailable; no compiler diagnostics were executed".into(),
        ),
    })
}

/// Helper to scan directory for code files excluding standard ignore folders.
/// Symlinks and junctions are never followed, and every file must resolve
/// beneath the canonical workspace `root`; whatever is left out for that
/// reason, or by a budget, is recorded in `incomplete`.
fn collect_code_files(
    dir: &Path,
    root: &Path,
    limit: usize,
    incomplete: &mut Incomplete,
) -> Vec<PathBuf> {
    let Ok(root) = fs::canonicalize(root) else {
        incomplete.note(INCOMPLETE_UNREADABLE);
        return Vec::new();
    };
    let mut results = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut entries_seen = 0usize;

    'walk: while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            incomplete.note(INCOMPLETE_UNREADABLE);
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                incomplete.note(INCOMPLETE_UNREADABLE);
                continue;
            };
            entries_seen += 1;
            if entries_seen > FALLBACK_MAX_ENTRIES {
                incomplete.note(INCOMPLETE_ENTRIES);
                break 'walk;
            }
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                incomplete.note(INCOMPLETE_UNREADABLE);
                continue;
            };
            if kind.is_symlink() && !fs::metadata(&path).is_ok_and(|meta| meta.is_file()) {
                // A directory link or junction (or a dangling one) is not followed.
                incomplete.note(INCOMPLETE_UNSAFE_ENTRY);
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if let Some(
                "rs" | "ts" | "js" | "py" | "go" | "c" | "cpp" | "h" | "hpp" | "java",
            ) = path.extension().and_then(|e| e.to_str())
            {
                if !fs::canonicalize(&path).is_ok_and(|real| real.starts_with(&root)) {
                    incomplete.note(INCOMPLETE_UNSAFE_ENTRY);
                    continue;
                }
                if results.len() >= limit {
                    incomplete.note(INCOMPLETE_FILES);
                    break 'walk;
                }
                results.push(path);
            }
        }
    }
    results
}

fn safe_workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let (outside_lexical, symlink_escape) = crate::check_path_boundary(cwd, &full);
    if outside_lexical || symlink_escape {
        return Err(format!(
            "Semantic fallback target {} is outside workspace root {}",
            full.display(),
            cwd.display()
        ));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn f10_capability_fallback() {
        assert_eq!(semantic_route(false, false, false), "text_fallback");
        assert_eq!(semantic_route(true, false, false), "unsupported");
        assert_eq!(semantic_route(true, true, true), "stale_requery");
        assert_eq!(semantic_route(true, true, false), "semantic");
    }

    #[test]
    fn test_no_server_installed_routes_to_text_fallback() {
        assert_eq!(semantic_route(false, true, false), "text_fallback");
        assert_eq!(semantic_route(false, false, true), "text_fallback");
    }

    #[test]
    fn test_unsupported_hierarchy() {
        // Server exists but does not support call hierarchy
        assert_eq!(semantic_route(true, false, false), "unsupported");
    }

    #[test]
    fn test_stale_server_triggers_stale_requery() {
        // Server is available and supports operation, but document state is stale
        assert_eq!(semantic_route(true, true, true), "stale_requery");
    }

    #[test]
    fn test_server_available_but_workspace_untrusted() {
        // Untrusted workspace forbids server launch: treat server_available as false
        let project_trusted = false;
        let server_binary_present = true;
        let effective_available = project_trusted && server_binary_present;
        assert_eq!(
            semantic_route(effective_available, true, false),
            "text_fallback"
        );
    }

    #[test]
    fn test_unknown_language_fallback() {
        // Unknown language has no configured server
        let supported_languages = ["rust", "typescript", "python"];
        let query_lang = "brainfuck";
        let has_server = supported_languages.contains(&query_lang);
        assert_eq!(semantic_route(has_server, true, false), "text_fallback");
    }

    #[test]
    fn text_fallback_rejects_missing_path_traversal() {
        let dir = tempdir().unwrap();
        let escaped = PathBuf::from("missing/../../escape.rs");
        let error = safe_workspace_path(dir.path(), &escaped).unwrap_err();
        assert!(error.contains("outside workspace root"));
    }

    #[test]
    fn test_text_fallback_cancellation() {
        let dir = tempdir().unwrap();
        let aborted = std::sync::atomic::AtomicBool::new(true);
        let res = text_fallback_definition(dir.path(), "my_symbol", None, Some(&aborted));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Operation aborted");
    }

    #[test]
    fn test_text_fallback_definition_and_references() {
        let dir = tempdir().unwrap();
        let code = r#"
pub struct Alpha;

impl Alpha {
    pub fn execute_job(&self) {
        println!("job done");
    }
}

fn helper() {
    let a = Alpha;
    a.execute_job();
}
"#;
        let file_path = dir.path().join("code.rs");
        fs::write(&file_path, code).unwrap();

        // Find definition of execute_job
        let def_res = text_fallback_definition(
            dir.path(),
            "execute_job",
            Some(&PathBuf::from("code.rs")),
            None,
        )
        .unwrap();
        assert_eq!(def_res.locations.len(), 1);
        assert!(def_res.locations[0].is_textual_fallback);
        assert_eq!(def_res.locations[0].path, "code.rs");
        assert_eq!(def_res.locations[0].range.start.line, 4);

        // Find references of execute_job
        let ref_res = text_fallback_references(
            dir.path(),
            "execute_job",
            Some(&PathBuf::from("code.rs")),
            None,
        )
        .unwrap();
        assert_eq!(ref_res.locations.len(), 2);
        assert!(ref_res.locations[0].is_textual_fallback);

        // Outline
        let outline_res = text_fallback_outline(dir.path(), "code.rs").unwrap();
        assert_eq!(outline_res.symbols.len(), 3);
        assert_eq!(outline_res.symbols[0].name, "Alpha");
        assert_eq!(outline_res.symbols[1].name, "execute_job");
        assert_eq!(outline_res.symbols[2].name, "helper");
    }

    #[test]
    fn text_fallback_definition_ignores_calls_on_declaration_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fixture.rs");
        fs::write(
            &file,
            "fn target() {}\nfn caller() { target(); }\nfn target_suffix() {}\n",
        )
        .unwrap();

        let result = text_fallback_definition(dir.path(), "target", Some(&file), None).unwrap();

        assert_eq!(result.locations.len(), 1);
        assert_eq!(result.locations[0].range.start.line, 0);
    }

    #[test]
    fn test_empty_legitimate_references() {
        // When text fallback finds nothing, it returns Ok with empty locations
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty.rs");
        fs::write(&file_path, "fn untouched() {}\n").unwrap();

        let ref_res = text_fallback_references(
            dir.path(),
            "nonexistent_symbol",
            Some(&PathBuf::from("empty.rs")),
            None,
        )
        .unwrap();
        assert_eq!(ref_res.locations.len(), 0);
        assert!(!ref_res.partial);
    }

    #[test]
    fn test_diagnostic_fallback_is_explicitly_partial() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();

        let result = text_fallback_diagnostics(dir.path(), "code.rs").unwrap();

        assert!(result.partial);
        assert!(result.diagnostics.is_empty());
        assert!(result
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no compiler diagnostics")));
    }

    #[test]
    fn semantic_fallback_rejects_targets_outside_the_workspace() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("outside.rs");
        fs::write(&outside_file, "fn outside() {}\n").unwrap();

        assert!(text_fallback_outline(workspace.path(), outside_file.to_str().unwrap()).is_err());
        assert!(
            text_fallback_diagnostics(workspace.path(), outside_file.to_str().unwrap()).is_err()
        );
        assert!(
            text_fallback_definition(workspace.path(), "outside", Some(&outside_file), None)
                .is_err()
        );
    }

    // ---- WP01: bounded, honest textual fallbacks ----

    fn range_of(location: &Location) -> (u32, u32, u32) {
        (
            location.range.start.line,
            location.range.start.character,
            location.range.end.character,
        )
    }

    #[test]
    fn audit_wp01_b008_outline_is_bounded_and_marked_partial() {
        let dir = tempdir().unwrap();
        // 1,000 declarations are complete; the 1,001st makes the result partial.
        let declarations = |count: usize| -> String {
            (0..count)
                .map(|index| format!("fn item_{index}() {{}}\n"))
                .collect()
        };
        fs::write(
            dir.path().join("exact.rs"),
            declarations(OUTLINE_MAX_SYMBOLS),
        )
        .unwrap();
        let exact = text_fallback_outline(dir.path(), "exact.rs").unwrap();
        assert_eq!(exact.symbols.len(), OUTLINE_MAX_SYMBOLS);
        assert!(!exact.partial);
        assert_eq!(exact.symbols[999].name, "item_999");
        assert_eq!(exact.symbols[999].range.start.line, 999);

        fs::write(
            dir.path().join("many.rs"),
            declarations(OUTLINE_MAX_SYMBOLS + 1),
        )
        .unwrap();
        let many = text_fallback_outline(dir.path(), "many.rs").unwrap();
        assert_eq!(many.symbols.len(), OUTLINE_MAX_SYMBOLS);
        assert!(many.partial);
        assert!(many.fallback_reason.unwrap().contains("1000 symbols"));

        // Source over the byte cap is read only up to its last whole line.
        let mut big = declarations(10);
        let filler = format!("// {}\n", "x".repeat(1022));
        while (big.len() as u64) <= FALLBACK_MAX_FILE_BYTES {
            big.push_str(&filler);
        }
        big.push_str("fn after_the_cap() {}\n");
        fs::write(dir.path().join("big.rs"), big).unwrap();
        let big = text_fallback_outline(dir.path(), "big.rs").unwrap();
        assert!(big.partial);
        assert_eq!(big.symbols.len(), 10);
        assert!(big
            .symbols
            .iter()
            .all(|symbol| symbol.name != "after_the_cap"));
        assert!(big.fallback_reason.unwrap().contains("4 MiB"));
    }

    /// Review finding 10: declaration columns come from the line itself, so
    /// trailing whitespace neither shifts them nor lands inside a character.
    #[test]
    fn audit_wp01_b009_declaration_columns_ignore_trailing_whitespace() {
        let dir = tempdir().unwrap();
        for (source, symbol, expected) in [
            // The panic case: a two-byte identifier and a trailing space.
            ("fn \u{e9}() {} \n", "\u{e9}", (0, 3, 4)),
            ("fn target() {}   \n", "target", (0, 3, 9)),
            ("fn target() {}\t\t\n", "target", (0, 3, 9)),
            ("    pub fn target() {} \n", "target", (0, 11, 17)),
            ("\u{3000}fn target() {}\u{3000}\n", "target", (0, 4, 10)),
            (
                "export default function caf\u{e9}() { /* \u{1F600} */ }  \n",
                "caf\u{e9}",
                (0, 24, 28),
            ),
            ("let \u{1F600} = 1; fn x() {}\n", "x", (0, 0, 0)),
        ] {
            let name = "d.rs";
            fs::write(dir.path().join(name), source).unwrap();
            let defined = text_fallback_definition(dir.path(), symbol, Some(Path::new(name)), None)
                .unwrap_or_else(|error| panic!("{source:?}: {error}"));
            if expected == (0, 0, 0) {
                // Not a declaration at the line start: no hit, and no panic.
                assert!(defined.locations.is_empty(), "{source:?}");
                continue;
            }
            assert_eq!(defined.locations.len(), 1, "{source:?}");
            assert_eq!(range_of(&defined.locations[0]), expected, "{source:?}");
        }
    }

    #[test]
    fn audit_wp01_b009_columns_are_utf16_code_units() {
        let dir = tempdir().unwrap();
        // "é" is 2 UTF-8 bytes / 1 UTF-16 unit; "😀" is 4 bytes / 2 units.
        fs::write(
            dir.path().join("a.rs"),
            "let é = 1; foo(); let 😀 = 2; foo(); foo();\n",
        )
        .unwrap();
        let found =
            text_fallback_references(dir.path(), "foo", Some(Path::new("a.rs")), None).unwrap();
        let columns: Vec<_> = found.locations.iter().map(range_of).collect();
        // "let é = 1; " = 11 units; "foo(); " = 7; "let 😀 = 2; " = 12 units.
        assert_eq!(columns, [(0, 11, 14), (0, 30, 33), (0, 37, 40)]);

        fs::write(dir.path().join("d.rs"), "/* é😀 */ fn target() {}\n").unwrap();
        let defined =
            text_fallback_definition(dir.path(), "target", Some(Path::new("d.rs")), None).unwrap();
        assert_eq!(
            defined.locations.len(),
            0,
            "a comment prefix is not a declaration"
        );
        fs::write(dir.path().join("e.rs"), "fn target() {}\n").unwrap();
        let defined =
            text_fallback_definition(dir.path(), "target", Some(Path::new("e.rs")), None).unwrap();
        assert_eq!(range_of(&defined.locations[0]), (0, 3, 9));

        fs::write(dir.path().join("o.rs"), "    pub fn café() {}\n").unwrap();
        let outline = text_fallback_outline(dir.path(), "o.rs").unwrap();
        let symbol = &outline.symbols[0];
        assert_eq!(symbol.name, "café");
        assert_eq!(symbol.range.end.character, 20);
        assert_eq!(
            (
                symbol.selection_range.start.character,
                symbol.selection_range.end.character
            ),
            (11, 15)
        );
    }

    #[test]
    fn audit_wp01_b010_outline_selection_is_the_identifier_only() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("a.ts"),
            "    pub fn alpha(x: u8) {}\n  function beta() {}\nclass Gamma {}\n",
        )
        .unwrap();
        let outline = text_fallback_outline(dir.path(), "a.ts").unwrap();
        let selected: Vec<_> = outline
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    symbol.selection_range.start.character,
                    symbol.selection_range.end.character,
                    symbol.range.start.character,
                    symbol.range.end.character,
                )
            })
            .collect();
        assert_eq!(
            selected,
            [
                ("alpha", 11, 16, 0, 26),
                ("beta", 11, 15, 0, 20),
                ("Gamma", 6, 11, 0, 14)
            ]
        );
    }

    #[test]
    fn audit_wp01_b011_and_b012_references_respect_identifier_boundaries() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "foo foobar _foo foo2 éfoo foo(foo) a.foo $foo\n",
        )
        .unwrap();
        let found =
            text_fallback_references(dir.path(), "foo", Some(Path::new("a.rs")), None).unwrap();
        let columns: Vec<_> = found.locations.iter().map(range_of).collect();
        // Alone (0), then `foo(foo)`: two on one line, then `a.foo`.
        // "foo foobar _foo foo2 éfoo " is 26 units.
        assert_eq!(columns, [(0, 0, 3), (0, 26, 29), (0, 30, 33), (0, 37, 40)]);
        assert!(!found.partial);

        fs::write(dir.path().join("b.rs"), "foo + foo\n").unwrap();
        let twice =
            text_fallback_references(dir.path(), "foo", Some(Path::new("b.rs")), None).unwrap();
        assert_eq!(
            twice.locations.iter().map(range_of).collect::<Vec<_>>(),
            [(0, 0, 3), (0, 6, 9)]
        );

        // The hit cap holds across lines and says it was reached.
        let many: String = (0..150).map(|_| "foo\n").collect();
        fs::write(dir.path().join("c.rs"), many).unwrap();
        let capped =
            text_fallback_references(dir.path(), "foo", Some(Path::new("c.rs")), None).unwrap();
        assert_eq!(capped.locations.len(), REFERENCE_MAX_HITS);
        assert!(capped.partial);
    }

    fn link_directory(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .is_ok_and(|out| out.status.success())
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    #[test]
    fn audit_wp01_b013_a_directory_link_to_outside_is_not_searched() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let canary_source = "fn canary() {}\ncanary();\n";
        fs::write(outside.path().join("canary.rs"), canary_source).unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        fs::write(
            workspace.path().join("src/real.rs"),
            "fn canary_real() {}\n",
        )
        .unwrap();
        if !link_directory(&workspace.path().join("escape"), outside.path()) {
            eprintln!("directory links are unavailable here: B013 link case not executed");
            return;
        }

        let definition = text_fallback_definition(workspace.path(), "canary", None, None).unwrap();
        assert!(
            definition.locations.is_empty(),
            "{:?}",
            definition.locations
        );
        assert!(
            definition.partial,
            "a skipped link makes the scan incomplete"
        );
        let references = text_fallback_references(workspace.path(), "canary", None, None).unwrap();
        assert!(references
            .locations
            .iter()
            .all(|l| !l.path.contains("escape")));
        assert!(references.partial);

        // A normal nested source file is still found.
        let real = text_fallback_definition(workspace.path(), "canary_real", None, None).unwrap();
        assert_eq!(real.locations.len(), 1);
        assert_eq!(real.locations[0].path, "src/real.rs");
        assert_eq!(
            fs::read_to_string(outside.path().join("canary.rs")).unwrap(),
            canary_source
        );
    }

    /// Review finding 5: the file or its directory is swapped for a link after
    /// discovery. The confinement check runs on the handle that is read, so the
    /// canary behind the link is refused, not scanned.
    #[test]
    fn audit_wp01_b013_a_link_swapped_in_after_discovery_is_refused_on_the_read_handle() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("a.rs"), "fn canary() {}\n").unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        let collected = workspace.path().join("src").join("a.rs");
        fs::write(&collected, "fn inside() {}\n").unwrap();
        // A collected, in-workspace file reads normally.
        let source = read_source(
            &AdmittedRoot::open(workspace.path()).unwrap(),
            &collected,
            false,
        )
        .unwrap();
        assert!(source.text.contains("inside"));

        // Replace the parent directory with a link to the outside directory.
        fs::remove_dir_all(workspace.path().join("src")).unwrap();
        if !link_directory(&workspace.path().join("src"), outside.path()) {
            eprintln!("directory links are unavailable here: swap case not executed");
            return;
        }
        // The pathname still looks workspace-relative and the file is a regular,
        // single-link file, but the handle resolves outside the workspace.
        assert!(collected.is_file());
        assert_eq!(
            read_source(
                &AdmittedRoot::open(workspace.path()).unwrap(),
                &collected,
                false
            )
            .err(),
            Some(INCOMPLETE_UNSAFE_ENTRY)
        );
        // Scanning through the public entry point never surfaces the canary.
        let found = text_fallback_definition(
            workspace.path(),
            "canary",
            Some(Path::new("src/a.rs")),
            None,
        );
        match found {
            Ok(result) => assert!(result.locations.is_empty(), "{:?}", result.locations),
            Err(error) => assert!(!error.contains("fn canary"), "{error}"),
        }
        let outline = text_fallback_outline(workspace.path(), "src/a.rs");
        if let Ok(outline) = outline {
            assert!(outline.symbols.is_empty(), "{:?}", outline.symbols);
        }
    }

    /// Re-check item 5: the workspace itself is renamed and a link to outside
    /// put at its name after discovery. The sources are judged against the
    /// workspace as admitted at the start, so nothing from outside surfaces
    /// through definition, references or outline. Where the platform refuses
    /// to rename an open directory (Windows) the swap cannot happen at all.
    #[test]
    fn audit_wp01_b013_a_replaced_workspace_root_admits_nothing_from_outside() {
        for which in ["definition", "references", "outline"] {
            let parent = tempdir().unwrap();
            let outside = tempdir().unwrap();
            let workspace = parent.path().join("ws");
            fs::create_dir_all(workspace.join("src")).unwrap();
            fs::write(workspace.join("src").join("a.rs"), "fn inside() {}\n").unwrap();
            fs::create_dir_all(outside.path().join("src")).unwrap();
            fs::write(
                outside.path().join("src").join("a.rs"),
                "fn canary() {}\ncanary();\n",
            )
            .unwrap();
            let moved = parent.path().join("ws-moved");
            let renamed = std::rc::Rc::new(std::cell::Cell::new(None::<bool>));
            let (ws, mv, out, flag) = (
                workspace.clone(),
                moved.clone(),
                outside.path().to_path_buf(),
                renamed.clone(),
            );
            SEMANTIC_BEFORE_READ.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |_| {
                    if flag.get().is_some() {
                        return;
                    }
                    let ok = fs::rename(&ws, &mv).is_ok();
                    flag.set(Some(ok));
                    if ok && !link_directory(&ws, &out) {
                        // No links here: restore and treat as not executed.
                        fs::rename(&mv, &ws).unwrap();
                        flag.set(Some(false));
                    }
                }));
            });
            let target = Path::new("src/a.rs");
            let leaked = match which {
                "definition" => text_fallback_definition(&workspace, "canary", Some(target), None)
                    .map(|r| !r.locations.is_empty())
                    .unwrap_or(false),
                "references" => text_fallback_references(&workspace, "canary", Some(target), None)
                    .map(|r| !r.locations.is_empty())
                    .unwrap_or(false),
                _ => text_fallback_outline(&workspace, "src/a.rs")
                    .map(|r| r.symbols.iter().any(|s| s.name.contains("canary")))
                    .unwrap_or(false),
            };
            SEMANTIC_BEFORE_READ.with(|hook| *hook.borrow_mut() = None);
            assert!(
                !leaked,
                "{which}: a symbol from outside the workspace surfaced"
            );
            if cfg!(windows) {
                assert_eq!(
                    renamed.get(),
                    Some(false),
                    "{which}: an open workspace must not be renamable"
                );
            }
        }
    }

    /// The admitted root is a fixed path: a source under any other tree is
    /// refused whatever the workspace name resolves to now (the part of the
    /// root-replacement race that needs no rename).
    #[test]
    fn audit_wp01_b013_sources_are_judged_against_the_root_as_admitted() {
        let admitted = tempdir().unwrap();
        let other = tempdir().unwrap();
        fs::write(
            other.path().join("a.rs"),
            "fn canary() {}
",
        )
        .unwrap();
        fs::write(
            admitted.path().join("a.rs"),
            "fn inside() {}
",
        )
        .unwrap();
        let root = AdmittedRoot::open(admitted.path()).unwrap();
        assert!(read_source(&root, &admitted.path().join("a.rs"), false).is_ok());
        assert_eq!(
            read_source(&root, &other.path().join("a.rs"), false).err(),
            Some(INCOMPLETE_UNSAFE_ENTRY)
        );
    }

    #[test]
    fn audit_wp01_b001_an_outside_read_grant_lets_the_fallback_scan_a_linked_file() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("o.rs"), "fn linked_symbol() {}\n").unwrap();
        fs::hard_link(outside.path().join("o.rs"), workspace.path().join("a.rs")).unwrap();
        let found = |allow| {
            text_fallback_definition_authorized(
                workspace.path(),
                "linked_symbol",
                Some(Path::new("a.rs")),
                None,
                allow,
            )
            .unwrap()
            .locations
            .len()
        };
        assert_eq!(found(false), 0, "restricted callers skip the linked file");
        assert_eq!(found(true), 1, "an authorized caller scans it");
        assert!(text_fallback_outline(workspace.path(), "a.rs").is_err());
        assert!(
            !text_fallback_outline_authorized(workspace.path(), "a.rs", true)
                .unwrap()
                .symbols
                .is_empty()
        );
    }

    #[test]
    fn audit_wp01_b016_capped_unreadable_and_exhausted_scans_are_partial() {
        let dir = tempdir().unwrap();
        // A complete small scan with no hit is complete.
        fs::write(dir.path().join("only.rs"), "fn other() {}\n").unwrap();
        let none = text_fallback_definition(dir.path(), "absent", None, None).unwrap();
        assert!(none.locations.is_empty() && !none.partial);

        // More than 50 definitions.
        let defs: String = (0..60).map(|_| "fn dup() {}\n").collect();
        fs::write(dir.path().join("dups.rs"), defs).unwrap();
        let capped = text_fallback_definition(dir.path(), "dup", None, None).unwrap();
        assert_eq!(capped.locations.len(), DEFINITION_MAX_HITS);
        assert!(capped.partial);

        // A file that is not readable text is skipped, and the result says so.
        fs::write(dir.path().join("binary.rs"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let skipped = text_fallback_definition(dir.path(), "absent", None, None).unwrap();
        assert!(skipped.partial);
        assert!(skipped.fallback_reason.unwrap().contains("unreadable"));

        // Discovery that runs past its file budget.
        let many = tempdir().unwrap();
        for index in 0..=FALLBACK_MAX_FILES {
            fs::write(many.path().join(format!("f{index}.rs")), "fn x() {}\n").unwrap();
        }
        let exhausted = text_fallback_definition(many.path(), "absent", None, None).unwrap();
        assert!(exhausted.partial);
        assert!(exhausted.fallback_reason.unwrap().contains("200 files"));
    }

    #[test]
    fn audit_wp01_b001_semantic_fallback_skips_a_hard_linked_source() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let canary = outside.path().join("secret.rs");
        fs::write(&canary, "fn secret_symbol() {}\n").unwrap();
        fs::hard_link(&canary, workspace.path().join("alias.rs")).unwrap();
        let found =
            text_fallback_definition(workspace.path(), "secret_symbol", None, None).unwrap();
        assert!(found.locations.is_empty());
        assert!(found.partial);
        assert!(found.fallback_reason.unwrap().contains("hard-linked"));
        assert!(text_fallback_outline(workspace.path(), "alias.rs").is_err());
    }
}

//! Jupyter notebooks (`.ipynb`) read as cells and edited by cell.
//!
//! No TypeScript counterpart: `pi` reads a notebook as the JSON it is. The
//! divergence is deliberate (phase 3 spec, "Notebooks"): a model that sees
//! `# [2] code` and the cell's output edits the right cell, while one that
//! sees `"source": ["import pandas as pd\n"]` rewrites JSON by hand. Files
//! are written back the way `nbformat` writes them — keys sorted, one-space
//! indentation unless the file used another — so a round trip leaves git
//! quiet.

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::edit_diff::{
    apply_edits_to_normalized_content, generate_diff_string, normalize_for_fuzzy_match,
    normalize_to_lf, Edit,
};

/// Lines an output may take before the rest is counted instead of shown.
const OUTPUT_LINES: usize = 20;

pub fn is_notebook_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ipynb"))
}

/// The notebook's JSON, when the text is one: an object with a `cells`
/// array. Anything else reads as plain text.
pub fn parse(text: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text).ok()?;
    value.get("cells")?.as_array()?;
    Some(value)
}

fn cells(notebook: &Value) -> &[Value] {
    notebook
        .get("cells")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn cells_mut(notebook: &mut Value) -> Option<&mut Vec<Value>> {
    notebook.get_mut("cells").and_then(Value::as_array_mut)
}

/// A cell's source as one string, whether the file stored it as a string
/// or as the usual array of lines.
pub fn cell_source(cell: &Value) -> String {
    text_of(cell.get("source"))
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Store a source the way nbformat does: one array element per line, each
/// keeping its newline.
fn source_value(source: &str) -> Value {
    if source.is_empty() {
        return Value::Array(Vec::new());
    }
    let mut lines: Vec<Value> = Vec::new();
    let mut rest = source;
    while let Some(index) = rest.find('\n') {
        lines.push(Value::String(rest[..=index].to_string()));
        rest = &rest[index + 1..];
    }
    if !rest.is_empty() {
        lines.push(Value::String(rest.to_string()));
    }
    Value::Array(lines)
}

fn cell_type(cell: &Value) -> &str {
    cell.get("cell_type")
        .and_then(Value::as_str)
        .unwrap_or("code")
}

/// Replace a cell's source. A code cell that changed has nothing to show
/// for its old outputs, so they go and the execution count with them.
pub fn set_cell_source(cell: &mut Value, source: &str) {
    let is_code = cell_type(cell) == "code";
    if let Some(object) = cell.as_object_mut() {
        object.insert("source".into(), source_value(source));
        if is_code {
            object.insert("outputs".into(), Value::Array(Vec::new()));
            object.insert("execution_count".into(), Value::Null);
        }
    }
}

/// nbformat 4.5 and later give every cell an `id`; Jupyter warns and
/// rewrites a notebook whose cells lack one.
fn wants_cell_ids(notebook: &Value) -> bool {
    notebook
        .get("nbformat")
        .and_then(Value::as_u64)
        .unwrap_or(4)
        > 4
        || notebook
            .get("nbformat_minor")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 5
}

fn new_cell(cell_type: &str, source: &str, with_id: bool) -> Value {
    let mut object = Map::new();
    if with_id {
        let id: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        object.insert("id".into(), Value::String(id));
    }
    object.insert("cell_type".into(), Value::String(cell_type.to_string()));
    object.insert("metadata".into(), Value::Object(Map::new()));
    object.insert("source".into(), source_value(source));
    if cell_type == "code" {
        object.insert("execution_count".into(), Value::Null);
        object.insert("outputs".into(), Value::Array(Vec::new()));
    }
    Value::Object(object)
}

/// `python`, `julia`, … from the kernel metadata; `code` when the file does
/// not say.
pub fn language(notebook: &Value) -> String {
    let metadata = notebook.get("metadata");
    metadata
        .and_then(|meta| meta.get("language_info"))
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .and_then(|meta| meta.get("kernelspec"))
                .and_then(|spec| spec.get("language"))
                .and_then(Value::as_str)
        })
        .unwrap_or("code")
        .to_string()
}

/// The notebook as the model reads it: a header, then every cell under a
/// `# [n] kind` line, code cells followed by their outputs under `# out:`.
pub fn render(notebook: &Value) -> String {
    let cells = cells(notebook);
    let mut out = vec![format!(
        "# notebook · {} · {}",
        plural(cells.len(), "cell"),
        language(notebook)
    )];
    for (index, cell) in cells.iter().enumerate() {
        out.push(format!("# [{}] {}", index + 1, cell_type(cell)));
        let source = cell_source(cell);
        for line in source.lines() {
            out.push(line.to_string());
        }
        if let Some(outputs) = cell.get("outputs").and_then(Value::as_array) {
            for output in outputs {
                out.extend(render_output(output));
            }
        }
    }
    out.join("\n")
}

fn plural(count: usize, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

fn render_output(output: &Value) -> Vec<String> {
    let kind = output
        .get("output_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let body: Vec<String> = match kind {
        "stream" => text_of(output.get("text"))
            .lines()
            .map(str::to_string)
            .collect(),
        "execute_result" | "display_data" => {
            let data = output.get("data");
            let text = text_of(data.and_then(|data| data.get("text/plain")));
            if text.is_empty() {
                let mime = data
                    .and_then(Value::as_object)
                    .and_then(|map| map.keys().find(|key| key.starts_with("image/")))
                    .cloned();
                match mime {
                    Some(mime) => vec![mime],
                    None => Vec::new(),
                }
            } else {
                text.lines().map(str::to_string).collect()
            }
        }
        "error" => {
            let name = output
                .get("ename")
                .and_then(Value::as_str)
                .unwrap_or("Error");
            let value = output.get("evalue").and_then(Value::as_str).unwrap_or("");
            let mut lines = vec![format!("{name}: {value}").trim().to_string()];
            if let Some(trace) = output.get("traceback").and_then(Value::as_array) {
                lines.extend(
                    trace
                        .iter()
                        .filter_map(Value::as_str)
                        .flat_map(|frame| frame.lines().map(strip_ansi).collect::<Vec<_>>()),
                );
            }
            lines
        }
        _ => Vec::new(),
    };
    if body.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for (index, line) in body.iter().take(OUTPUT_LINES).enumerate() {
        if index == 0 {
            rows.push(format!("# out: {line}"));
        } else {
            rows.push(format!("#      {line}"));
        }
    }
    if body.len() > OUTPUT_LINES {
        rows.push(format!("#      … {} more lines", body.len() - OUTPUT_LINES));
    }
    rows
}

/// Tracebacks arrive coloured; the model reads them better plain.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// One changed cell of an in-cell edit: which one and its before and after.
#[derive(Debug, Clone)]
pub struct CellChange {
    pub index: usize,
    pub before: String,
    pub after: String,
}

/// Apply `edit`-tool replacements inside cell sources. Each edit must match
/// exactly one cell; edits to the same cell are applied together with the
/// plain-text engine so the usual overlap and no-change errors still hold.
pub fn edit_in_cells(
    notebook: &mut Value,
    edits: &[Edit],
    display_path: &str,
) -> Result<Vec<CellChange>, String> {
    let sources: Vec<String> = cells(notebook)
        .iter()
        .map(|cell| normalize_to_lf(&cell_source(cell)))
        .collect();
    let mut per_cell: Vec<(usize, Vec<Edit>)> = Vec::new();
    for (edit_index, edit) in edits.iter().enumerate() {
        let needle = normalize_for_fuzzy_match(&normalize_to_lf(&edit.old_text));
        if needle.is_empty() {
            return Err(format!("oldText must not be empty in {display_path}."));
        }
        let hits: Vec<usize> = sources
            .iter()
            .enumerate()
            .filter(|(_, source)| normalize_for_fuzzy_match(source).contains(&needle))
            .map(|(index, _)| index)
            .collect();
        match hits.len() {
            0 => {
                return Err(format!(
                    "Could not find the exact text in {display_path} (searched {}; edits[{edit_index}]). The old text must match exactly including all whitespace and newlines.",
                    plural(sources.len(), "cell")
                ))
            }
            1 => {}
            many => {
                return Err(format!(
                    "Found edits[{edit_index}] in {many} cells of {display_path}. The text must be unique to one cell. Please provide more context to make it unique."
                ))
            }
        }
        let cell = hits[0];
        match per_cell.iter_mut().find(|(index, _)| *index == cell) {
            Some((_, list)) => list.push(edit.clone()),
            None => per_cell.push((cell, vec![edit.clone()])),
        }
    }
    let mut changes = Vec::new();
    for (index, cell_edits) in per_cell {
        let before = sources[index].clone();
        let applied = apply_edits_to_normalized_content(
            &before,
            &cell_edits,
            &format!("{display_path} cell {}", index + 1),
        )?;
        changes.push(CellChange {
            index,
            before,
            after: applied.new_content,
        });
    }
    if let Some(cells) = cells_mut(notebook) {
        for change in &changes {
            if let Some(cell) = cells.get_mut(change.index) {
                set_cell_source(cell, &change.after);
            }
        }
    }
    Ok(changes)
}

/// The display diff of a set of cell changes: each cell's diff under a
/// `cell n` line, so a Δ block can say which cell moved.
pub fn changes_diff(changes: &[CellChange]) -> (String, Option<usize>) {
    let mut out = Vec::new();
    let mut first = None;
    for change in changes {
        let (diff, changed) = generate_diff_string(&change.before, &change.after, 4);
        if first.is_none() {
            first = changed;
        }
        if changes.len() > 1 {
            out.push(format!("cell {}", change.index + 1));
        }
        out.push(diff);
    }
    (out.join("\n"), first)
}

/// What `notebook_edit` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Replace,
    Insert,
    Delete,
}

impl EditMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "replace" => Some(Self::Replace),
            "insert" => Some(Self::Insert),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// The outcome of a structural edit, for the tool result and the Δ block.
#[derive(Debug, Clone)]
pub struct StructuralEdit {
    pub summary: String,
    pub diff: String,
    pub cells: usize,
}

/// Replace, insert after, or delete one cell. `cell` is 1-based; for
/// `insert`, 0 puts the new cell first.
pub fn structural_edit(
    notebook: &mut Value,
    display_path: &str,
    cell: usize,
    mode: EditMode,
    source: Option<&str>,
    cell_type: Option<&str>,
) -> Result<StructuralEdit, String> {
    let count = cells(notebook).len();
    let file = Path::new(display_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_path.to_string());
    match mode {
        EditMode::Replace => {
            if cell == 0 || cell > count {
                return Err(out_of_range(cell, count, display_path));
            }
            let source = source.ok_or_else(|| {
                "notebook_edit replace needs `source` — the cell's new content.".to_string()
            })?;
            let cells = cells_mut(notebook).ok_or("not a notebook")?;
            let target = &mut cells[cell - 1];
            let before = normalize_to_lf(&cell_source(target));
            if let Some(kind) = cell_type {
                if let Some(object) = target.as_object_mut() {
                    object.insert("cell_type".into(), Value::String(kind.to_string()));
                    if kind != "code" {
                        object.remove("outputs");
                        object.remove("execution_count");
                    }
                }
            }
            set_cell_source(target, source);
            let (diff, _) = generate_diff_string(&before, &normalize_to_lf(source), 4);
            Ok(StructuralEdit {
                summary: format!("Replaced cell {cell} of {file}{}", first_line_note(source)),
                diff,
                cells: count,
            })
        }
        EditMode::Insert => {
            if cell > count {
                return Err(out_of_range(cell, count, display_path));
            }
            let source = source.ok_or_else(|| {
                "notebook_edit insert needs `source` — the new cell's content.".to_string()
            })?;
            let kind = cell_type.unwrap_or("code");
            let with_id = wants_cell_ids(notebook);
            let cells = cells_mut(notebook).ok_or("not a notebook")?;
            cells.insert(cell, new_cell(kind, source, with_id));
            let (diff, _) = generate_diff_string("", &normalize_to_lf(source), 4);
            Ok(StructuralEdit {
                summary: format!(
                    "Inserted {kind} cell {} of {file}{}",
                    cell + 1,
                    first_line_note(source)
                ),
                diff,
                cells: count + 1,
            })
        }
        EditMode::Delete => {
            if cell == 0 || cell > count {
                return Err(out_of_range(cell, count, display_path));
            }
            let cells = cells_mut(notebook).ok_or("not a notebook")?;
            let removed = cells.remove(cell - 1);
            let before = normalize_to_lf(&cell_source(&removed));
            let (diff, _) = generate_diff_string(&before, "", 4);
            Ok(StructuralEdit {
                summary: format!("Deleted cell {cell} of {file}"),
                diff,
                cells: count - 1,
            })
        }
    }
}

fn first_line_note(source: &str) -> String {
    match source
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        Some(line) => format!("\n{line}"),
        None => String::new(),
    }
}

fn out_of_range(cell: usize, count: usize, path: &str) -> String {
    format!(
        "Cell {cell} is out of range: {path} has {}.",
        plural(count, "cell")
    )
}

/// The indentation the file used, so writing it back changes only what the
/// edit changed. nbformat's own default is one space.
pub fn detect_indent(text: &str) -> usize {
    text.lines()
        .skip(1)
        .find(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .filter(|width| *width > 0)
        .unwrap_or(1)
}

/// The notebook as nbformat writes it: sorted keys, the file's indentation,
/// a trailing newline.
pub fn serialize(notebook: &Value, indent: usize) -> String {
    let unit = " ".repeat(indent.clamp(1, 8));
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(notebook, &mut serializer).ok();
    let mut text = String::from_utf8(out).unwrap_or_default();
    text.push('\n');
    text
}

pub fn apply_kind(kind: Option<&str>) -> Result<Option<&str>, String> {
    match kind.map(str::trim) {
        None | Some("") => Ok(None),
        Some(kind @ ("code" | "markdown" | "raw")) => Ok(Some(kind)),
        Some(other) => Err(format!(
            "cellType must be code, markdown or raw, not `{other}`."
        )),
    }
}

/// `{"type":"object", …}` for `notebook_edit`.
pub fn tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Path to the .ipynb file"},
            "cell": {"type": "integer", "description": "1-based cell number. For insert, the new cell goes after this one; 0 inserts at the top."},
            "mode": {"type": "string", "enum": ["replace", "insert", "delete"], "description": "replace the cell's source, insert a new cell after it, or delete it"},
            "source": {"type": "string", "description": "The cell's new source (replace, insert)"},
            "cellType": {"type": "string", "enum": ["code", "markdown", "raw"], "description": "Cell type for insert (default code) or to change on replace"}
        },
        "required": ["path", "cell", "mode"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notebook() -> Value {
        json!({
            "cells": [
                {"cell_type": "markdown", "metadata": {}, "source": ["# Title\n", "intro"]},
                {
                    "cell_type": "code", "execution_count": 3, "metadata": {},
                    "source": "import pandas as pd\ndf = pd.DataFrame()",
                    "outputs": [
                        {"output_type": "stream", "name": "stdout", "text": ["hello\n", "world\n"]},
                        {"output_type": "execute_result", "data": {"text/plain": ["<DataFrame 3×2>"]}, "execution_count": 3, "metadata": {}},
                        {"output_type": "display_data", "data": {"image/png": "iVBOR..."}, "metadata": {}},
                        {"output_type": "error", "ename": "ValueError", "evalue": "bad", "traceback": ["[31mTraceback[0m", "  line 2"]}
                    ]
                }
            ],
            "metadata": {"kernelspec": {"language": "python", "name": "python3"}, "language_info": {"name": "python"}},
            "nbformat": 4, "nbformat_minor": 5
        })
    }

    #[test]
    fn a_notebook_reads_as_numbered_cells_with_their_outputs() {
        let text = render(&notebook());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "# notebook · 2 cells · python");
        assert_eq!(lines[1], "# [1] markdown");
        assert_eq!(lines[2], "# Title");
        assert_eq!(lines[3], "intro");
        assert_eq!(lines[4], "# [2] code");
        assert_eq!(lines[5], "import pandas as pd");
        assert_eq!(lines[7], "# out: hello");
        assert_eq!(lines[8], "#      world");
        assert_eq!(lines[9], "# out: <DataFrame 3×2>");
        assert_eq!(lines[10], "# out: image/png");
        assert_eq!(lines[11], "# out: ValueError: bad");
        assert_eq!(lines[12], "#      Traceback");
    }

    #[test]
    fn long_outputs_are_counted_past_twenty_lines() {
        let mut text = String::new();
        for n in 1..=30 {
            text.push_str("row ");
            text.push_str(&n.to_string());
            text.push('\n');
        }
        let output = json!({"output_type": "stream", "text": text});
        let rows = render_output(&output);
        assert_eq!(rows.len(), 21);
        assert_eq!(rows[20], "#      … 10 more lines");
    }

    #[test]
    fn an_edit_lands_in_the_one_cell_that_holds_the_text_and_clears_its_outputs() {
        let mut nb = notebook();
        let changes = edit_in_cells(
            &mut nb,
            &[Edit {
                old_text: "pd.DataFrame()".into(),
                new_text: "pd.DataFrame({'a': [1]})".into(),
            }],
            "nb.ipynb",
        )
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].index, 1);
        let cell = &nb["cells"][1];
        assert_eq!(
            cell_source(cell),
            "import pandas as pd\ndf = pd.DataFrame({'a': [1]})"
        );
        assert_eq!(cell["outputs"], json!([]));
        assert_eq!(cell["execution_count"], Value::Null);
        assert_eq!(
            cell["source"],
            json!(["import pandas as pd\n", "df = pd.DataFrame({'a': [1]})"])
        );
        let (diff, first) = changes_diff(&changes);
        assert!(diff.contains("-2 df = pd.DataFrame()"), "{diff}");
        assert!(diff.contains("+2 df = pd.DataFrame({'a': [1]})"), "{diff}");
        assert_eq!(first, Some(2));
    }

    #[test]
    fn an_edit_that_matches_no_cell_or_several_is_refused() {
        let mut nb = notebook();
        let missing = edit_in_cells(
            &mut nb,
            &[Edit {
                old_text: "nowhere".into(),
                new_text: "x".into(),
            }],
            "nb.ipynb",
        )
        .unwrap_err();
        assert!(
            missing.starts_with("Could not find the exact text in nb.ipynb (searched 2 cells"),
            "{missing}"
        );
        nb["cells"][0]["source"] = json!("import pandas as pd");
        let twice = edit_in_cells(
            &mut nb,
            &[Edit {
                old_text: "import pandas".into(),
                new_text: "import polars".into(),
            }],
            "nb.ipynb",
        )
        .unwrap_err();
        assert!(twice.contains("in 2 cells of nb.ipynb"), "{twice}");
    }

    #[test]
    fn structural_edits_replace_insert_and_delete_cells() {
        let mut nb = notebook();
        let replaced = structural_edit(
            &mut nb,
            "nb.ipynb",
            1,
            EditMode::Replace,
            Some("# New title\n"),
            None,
        )
        .unwrap();
        assert_eq!(replaced.summary, "Replaced cell 1 of nb.ipynb\n# New title");
        assert_eq!(cell_source(&nb["cells"][0]), "# New title\n");
        assert!(replaced.diff.contains("-1 # Title"));

        let inserted = structural_edit(
            &mut nb,
            "nb.ipynb",
            0,
            EditMode::Insert,
            Some("print('first')"),
            Some("code"),
        )
        .unwrap();
        assert_eq!(
            inserted.summary,
            "Inserted code cell 1 of nb.ipynb\nprint('first')"
        );
        assert_eq!(inserted.cells, 3);
        assert_eq!(nb["cells"][0]["cell_type"], "code");
        assert_eq!(nb["cells"][0]["outputs"], json!([]));

        let deleted =
            structural_edit(&mut nb, "nb.ipynb", 3, EditMode::Delete, None, None).unwrap();
        assert_eq!(deleted.summary, "Deleted cell 3 of nb.ipynb");
        assert_eq!(cells(&nb).len(), 2);
        assert!(deleted.diff.contains("-1 import pandas as pd"));

        let out =
            structural_edit(&mut nb, "nb.ipynb", 9, EditMode::Delete, None, None).unwrap_err();
        assert_eq!(out, "Cell 9 is out of range: nb.ipynb has 2 cells.");
        let no_source =
            structural_edit(&mut nb, "nb.ipynb", 1, EditMode::Replace, None, None).unwrap_err();
        assert!(no_source.contains("needs `source`"));
    }

    #[test]
    fn a_notebook_is_written_the_way_nbformat_writes_it() {
        let text = "{\n \"cells\": [],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n";
        assert_eq!(detect_indent(text), 1);
        let nb = parse(text).unwrap();
        assert_eq!(serialize(&nb, detect_indent(text)), text);
        assert_eq!(detect_indent("{\n    \"cells\": []\n}"), 4);
        assert!(parse("{\"not\": \"a notebook\"}").is_none());
        assert!(parse("plain text").is_none());
    }
}

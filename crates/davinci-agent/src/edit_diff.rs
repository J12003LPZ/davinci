//! Edit matching matching `vendor/pi/packages/coding-agent/src/core/tools/edit-diff.ts`.

#[derive(Debug, Clone)]
pub struct Edit {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone)]
pub struct AppliedEdits {
    pub base_content: String,
    pub new_content: String,
}

#[derive(Debug, Clone)]
struct TextReplacement {
    edit_index: usize,
    match_index: usize,
    match_length: usize,
    new_text: String,
}

#[derive(Debug, Clone)]
struct LineSpan {
    start: usize,
    end: usize,
}

pub fn detect_line_ending(content: &str) -> &'static str {
    let crlf = content.find("\r\n");
    let lf = content.find('\n');
    match (crlf, lf) {
        (None, None) | (None, Some(_)) => "\n",
        (Some(_), None) => "\r\n",
        (Some(crlf), Some(lf)) => {
            if crlf < lf {
                "\r\n"
            } else {
                "\n"
            }
        }
    }
}

pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn restore_line_endings(text: &str, ending: &str) -> String {
    if ending == "\r\n" {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

pub fn split_bom(raw: &str) -> (&str, &str) {
    if let Some(rest) = raw.strip_prefix('\u{feff}') {
        ("\u{feff}", rest)
    } else {
        ("", raw)
    }
}

pub fn normalize_for_fuzzy_match(text: &str) -> String {
    let stripped = text
        .split('\n')
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    stripped
        .replace(['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'], "'")
        .replace(['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'], "\"")
        .replace(
            [
                '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
            ],
            "-",
        )
        .replace(
            [
                '\u{00A0}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
                '\u{2008}', '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}',
            ],
            " ",
        )
}

fn fuzzy_find_text(content: &str, old_text: &str) -> Option<(usize, usize, bool, String)> {
    if let Some(index) = content.find(old_text) {
        return Some((index, old_text.len(), false, content.to_string()));
    }
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    fuzzy_content
        .find(&fuzzy_old)
        .map(|index| (index, fuzzy_old.len(), true, fuzzy_content))
}

fn count_occurrences(content: &str, old_text: &str) -> usize {
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    if fuzzy_old.is_empty() {
        return 0;
    }
    fuzzy_content.matches(&fuzzy_old).count()
}

fn not_found_error(path: &str, edit_index: usize, total: usize) -> String {
    if total == 1 {
        format!(
            "Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines."
        )
    } else {
        format!(
            "Could not find edits[{edit_index}] in {path}. The oldText must match exactly including all whitespace and newlines."
        )
    }
}

fn duplicate_error(path: &str, edit_index: usize, total: usize, occurrences: usize) -> String {
    if total == 1 {
        format!(
            "Found {occurrences} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique."
        )
    } else {
        format!(
            "Found {occurrences} occurrences of edits[{edit_index}] in {path}. Each oldText must be unique. Please provide more context to make it unique."
        )
    }
}

fn empty_old_text_error(path: &str, edit_index: usize, total: usize) -> String {
    if total == 1 {
        format!("oldText must not be empty in {path}.")
    } else {
        format!("edits[{edit_index}].oldText must not be empty in {path}.")
    }
}

fn no_change_error(path: &str, total: usize) -> String {
    if total == 1 {
        format!(
            "No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."
        )
    } else {
        format!("No changes made to {path}. The replacements produced identical content.")
    }
}

fn split_lines_with_endings(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (idx, ch) in content.char_indices() {
        if ch == '\n' {
            lines.push(content[start..=idx].to_string());
            start = idx + 1;
        }
    }
    if start < content.len() {
        lines.push(content[start..].to_string());
    }
    lines
}

fn get_line_spans(content: &str) -> Vec<LineSpan> {
    let mut offset = 0usize;
    split_lines_with_endings(content)
        .into_iter()
        .map(|line| {
            let span = LineSpan {
                start: offset,
                end: offset + line.len(),
            };
            offset = span.end;
            span
        })
        .collect()
}

fn replacement_line_range(
    lines: &[LineSpan],
    replacement: &TextReplacement,
) -> Result<(usize, usize), String> {
    let start = replacement.match_index;
    let end = replacement.match_index + replacement.match_length;
    let start_line = lines
        .iter()
        .position(|line| start >= line.start && start < line.end)
        .ok_or_else(|| "Replacement range is outside the base content.".to_string())?;
    let mut end_line = start_line;
    while end_line < lines.len() && lines[end_line].end < end {
        end_line += 1;
    }
    if end_line >= lines.len() {
        return Err("Replacement range is outside the base content.".into());
    }
    Ok((start_line, end_line + 1))
}

fn apply_replacements(content: &str, replacements: &[TextReplacement], offset: usize) -> String {
    let mut result = content.to_string();
    for replacement in replacements.iter().rev() {
        let match_index = replacement.match_index.saturating_sub(offset);
        let end = match_index + replacement.match_length;
        if match_index > result.len() || end > result.len() {
            continue;
        }
        result = format!(
            "{}{}{}",
            &result[..match_index],
            replacement.new_text,
            &result[end..]
        );
    }
    result
}

fn apply_replacements_preserving_unchanged_lines(
    original_content: &str,
    base_content: &str,
    replacements: &[TextReplacement],
) -> Result<String, String> {
    let original_lines = split_lines_with_endings(original_content);
    let base_lines = get_line_spans(base_content);
    if original_lines.len() != base_lines.len() {
        return Err(
            "Cannot preserve unchanged lines because the base content has a different line count."
                .into(),
        );
    }
    let mut groups: Vec<(usize, usize, Vec<TextReplacement>)> = Vec::new();
    let mut sorted = replacements.to_vec();
    sorted.sort_by_key(|item| item.match_index);
    for replacement in sorted {
        let (start_line, end_line) = replacement_line_range(&base_lines, &replacement)?;
        if let Some(current) = groups.last_mut() {
            if start_line < current.1 {
                current.1 = current.1.max(end_line);
                current.2.push(replacement);
                continue;
            }
        }
        groups.push((start_line, end_line, vec![replacement]));
    }
    let mut original_line_index = 0usize;
    let mut result = String::new();
    for (start_line, end_line, group_replacements) in groups {
        result.push_str(&original_lines[original_line_index..start_line].join(""));
        let group_start = base_lines[start_line].start;
        let group_end = base_lines[end_line - 1].end;
        result.push_str(&apply_replacements(
            &base_content[group_start..group_end],
            &group_replacements,
            group_start,
        ));
        original_line_index = end_line;
    }
    result.push_str(&original_lines[original_line_index..].join(""));
    Ok(result)
}

pub fn apply_edits_to_normalized_content(
    normalized_content: &str,
    edits: &[Edit],
    path: &str,
) -> Result<AppliedEdits, String> {
    let normalized_edits: Vec<Edit> = edits
        .iter()
        .map(|edit| Edit {
            old_text: normalize_to_lf(&edit.old_text),
            new_text: normalize_to_lf(&edit.new_text),
        })
        .collect();
    for (index, edit) in normalized_edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(empty_old_text_error(path, index, normalized_edits.len()));
        }
    }
    let initial: Vec<_> = normalized_edits
        .iter()
        .map(|edit| fuzzy_find_text(normalized_content, &edit.old_text))
        .collect();
    let used_fuzzy = initial.iter().any(|item| {
        item.as_ref()
            .is_some_and(|(_, _, used_fuzzy, _)| *used_fuzzy)
    });
    let replacement_base = if used_fuzzy {
        normalize_for_fuzzy_match(normalized_content)
    } else {
        normalized_content.to_string()
    };
    let mut matched = Vec::new();
    for (index, edit) in normalized_edits.iter().enumerate() {
        let Some((match_index, match_length, _, _)) =
            fuzzy_find_text(&replacement_base, &edit.old_text)
        else {
            return Err(not_found_error(path, index, normalized_edits.len()));
        };
        let occurrences = count_occurrences(&replacement_base, &edit.old_text);
        if occurrences > 1 {
            return Err(duplicate_error(
                path,
                index,
                normalized_edits.len(),
                occurrences,
            ));
        }
        matched.push(TextReplacement {
            edit_index: index,
            match_index,
            match_length,
            new_text: edit.new_text.clone(),
        });
    }
    matched.sort_by_key(|item| item.match_index);
    for window in matched.windows(2) {
        let previous = &window[0];
        let current = &window[1];
        if previous.match_index + previous.match_length > current.match_index {
            return Err(format!(
                "edits[{}] and edits[{}] overlap in {path}. Merge them into one edit or target disjoint regions.",
                previous.edit_index, current.edit_index
            ));
        }
    }
    let new_content = if used_fuzzy {
        apply_replacements_preserving_unchanged_lines(
            normalized_content,
            &replacement_base,
            &matched,
        )?
    } else {
        apply_replacements(&replacement_base, &matched, 0)
    };
    if normalized_content == new_content {
        return Err(no_change_error(path, normalized_edits.len()));
    }
    Ok(AppliedEdits {
        base_content: normalized_content.to_string(),
        new_content,
    })
}

pub fn prepare_edit_arguments(input: &serde_json::Value) -> Result<(String, Vec<Edit>), String> {
    let Some(object) = input.as_object() else {
        return Err(
            "Edit tool input is invalid. edits must contain at least one replacement.".into(),
        );
    };
    let path = object
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let mut edits = Vec::new();
    match object.get("edits") {
        Some(serde_json::Value::String(raw)) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
                collect_edits(&parsed, &mut edits);
            }
        }
        Some(value) => collect_edits(value, &mut edits),
        None => {}
    }
    if let (Some(old), Some(new)) = (
        object.get("oldText").and_then(|value| value.as_str()),
        object.get("newText").and_then(|value| value.as_str()),
    ) {
        edits.push(Edit {
            old_text: old.to_string(),
            new_text: new.to_string(),
        });
    }
    if edits.is_empty() {
        return Err(
            "Edit tool input is invalid. edits must contain at least one replacement.".into(),
        );
    }
    Ok((path, edits))
}

fn collect_edits(value: &serde_json::Value, edits: &mut Vec<Edit>) {
    if let Some(items) = value.as_array() {
        for item in items {
            if let Some(edit) = single_edit(item) {
                edits.push(edit);
            }
        }
    } else if let Some(edit) = single_edit(value) {
        edits.push(edit);
    }
}

fn single_edit(value: &serde_json::Value) -> Option<Edit> {
    Some(Edit {
        old_text: value.get("oldText")?.as_str()?.to_string(),
        new_text: value.get("newText")?.as_str()?.to_string(),
    })
}

/// One run of a line diff, grouped as `Diff.diffLines` groups them: every
/// consecutive line of the same kind in one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPart {
    pub kind: DiffKind,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Equal,
    Added,
    Removed,
}

/// Lines of a text for diffing: a trailing newline does not make an empty
/// last line, and `\r\n` is `\n`.
fn diff_source_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Above this many lines the O(ND) trace is not worth its memory; the whole
/// old text is reported removed and the new one added.
const MYERS_LINE_LIMIT: usize = 40_000;

/// A line diff (Myers, O(ND)), the pieces grouped like the `diff` package's
/// `diffLines` so `generate_diff_string` can mirror the TypeScript function
/// line for line.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffPart> {
    let a = diff_source_lines(old);
    let b = diff_source_lines(new);
    let ops = if a.len() + b.len() > MYERS_LINE_LIMIT {
        let mut ops: Vec<(DiffKind, usize)> =
            (0..a.len()).map(|i| (DiffKind::Removed, i)).collect();
        ops.extend((0..b.len()).map(|j| (DiffKind::Added, j)));
        ops
    } else {
        myers(&a, &b)
    };
    let mut parts: Vec<DiffPart> = Vec::new();
    for (kind, index) in ops {
        let line = match kind {
            DiffKind::Added => b[index],
            DiffKind::Equal | DiffKind::Removed => a[index],
        };
        match parts.last_mut() {
            Some(part) if part.kind == kind => part.lines.push(line.to_string()),
            _ => parts.push(DiffPart {
                kind,
                lines: vec![line.to_string()],
            }),
        }
    }
    parts
}

/// The classic Myers walk with a trace of every furthest-reaching frontier,
/// walked back to recover the edit script. Equal lines carry their index in
/// `a`; added lines their index in `b`.
fn myers(a: &[&str], b: &[&str]) -> Vec<(DiffKind, usize)> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    let max = n + m;
    if max == 0 {
        return Vec::new();
    }
    let offset = max;
    let width = (2 * max + 1) as usize;
    let mut v = vec![0isize; width];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = true;
                break;
            }
            k += 2;
        }
        if found {
            break;
        }
    }
    // Walk the trace back from the end, emitting the script in reverse.
    let mut script: Vec<(DiffKind, usize)> = Vec::new();
    let mut x = n;
    let mut y = m;
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let index = (k + offset) as usize;
        let prev_k = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            script.push((DiffKind::Equal, x as usize));
        }
        if d > 0 {
            if x == prev_x {
                y -= 1;
                script.push((DiffKind::Added, y as usize));
            } else {
                x -= 1;
                script.push((DiffKind::Removed, x as usize));
            }
        }
    }
    script.reverse();
    script
}

/// The display diff of an edit, exactly as TypeScript's `generateDiffString`
/// lays it out: `+NN text` / `-NN text` / ` NN text`, four lines of context
/// each side, `...` where context is skipped, and the first changed line in
/// the new file.
pub fn generate_diff_string(old: &str, new: &str, context_lines: usize) -> (String, Option<usize>) {
    let parts = diff_lines(old, new);
    let old_total = old.split('\n').count();
    let new_total = new.split('\n').count();
    let width = old_total.max(new_total).to_string().len();
    let number = |n: usize| format!("{n:>width$}");
    let blank = " ".repeat(width);
    let mut output: Vec<String> = Vec::new();
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut last_was_change = false;
    let mut first_changed = None;
    for (index, part) in parts.iter().enumerate() {
        let raw = &part.lines;
        match part.kind {
            DiffKind::Added | DiffKind::Removed => {
                if first_changed.is_none() {
                    first_changed = Some(new_line);
                }
                for line in raw {
                    if part.kind == DiffKind::Added {
                        output.push(format!("+{} {line}", number(new_line)));
                        new_line += 1;
                    } else {
                        output.push(format!("-{} {line}", number(old_line)));
                        old_line += 1;
                    }
                }
                last_was_change = true;
            }
            DiffKind::Equal => {
                let next_is_change = parts
                    .get(index + 1)
                    .is_some_and(|next| next.kind != DiffKind::Equal);
                let context = |output: &mut Vec<String>,
                               line: &str,
                               old_line: &mut usize,
                               new_line: &mut usize| {
                    output.push(format!(" {} {line}", number(*old_line)));
                    *old_line += 1;
                    *new_line += 1;
                };
                if last_was_change && next_is_change {
                    if raw.len() <= context_lines * 2 {
                        for line in raw {
                            context(&mut output, line, &mut old_line, &mut new_line);
                        }
                    } else {
                        let skipped = raw.len() - context_lines * 2;
                        for line in &raw[..context_lines] {
                            context(&mut output, line, &mut old_line, &mut new_line);
                        }
                        output.push(format!(" {blank} ..."));
                        old_line += skipped;
                        new_line += skipped;
                        for line in &raw[raw.len() - context_lines..] {
                            context(&mut output, line, &mut old_line, &mut new_line);
                        }
                    }
                } else if last_was_change {
                    let shown = raw.len().min(context_lines);
                    for line in &raw[..shown] {
                        context(&mut output, line, &mut old_line, &mut new_line);
                    }
                    let skipped = raw.len() - shown;
                    if skipped > 0 {
                        output.push(format!(" {blank} ..."));
                        old_line += skipped;
                        new_line += skipped;
                    }
                } else if next_is_change {
                    let skipped = raw.len().saturating_sub(context_lines);
                    if skipped > 0 {
                        output.push(format!(" {blank} ..."));
                        old_line += skipped;
                        new_line += skipped;
                    }
                    for line in &raw[skipped..] {
                        context(&mut output, line, &mut old_line, &mut new_line);
                    }
                } else {
                    old_line += raw.len();
                    new_line += raw.len();
                }
                last_was_change = false;
            }
        }
    }
    (output.join("\n"), first_changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_diff_finds_inserts_deletes_and_replacements() {
        let kinds = |old: &str, new: &str| -> Vec<(DiffKind, usize)> {
            diff_lines(old, new)
                .into_iter()
                .map(|part| (part.kind, part.lines.len()))
                .collect()
        };
        assert_eq!(kinds("a\nb\n", "a\nb\n"), vec![(DiffKind::Equal, 2)]);
        assert_eq!(
            kinds("a\nc\n", "a\nb\nc\n"),
            vec![
                (DiffKind::Equal, 1),
                (DiffKind::Added, 1),
                (DiffKind::Equal, 1)
            ]
        );
        assert_eq!(
            kinds("a\nb\nc\n", "a\nc\n"),
            vec![
                (DiffKind::Equal, 1),
                (DiffKind::Removed, 1),
                (DiffKind::Equal, 1)
            ]
        );
        let replaced = diff_lines("one\ntwo\nthree\n", "one\n2\nthree\n");
        assert_eq!(replaced.len(), 4);
        assert_eq!(replaced[1].kind, DiffKind::Removed);
        assert_eq!(replaced[1].lines, vec!["two"]);
        assert_eq!(replaced[2].kind, DiffKind::Added);
        assert_eq!(replaced[2].lines, vec!["2"]);
        assert!(diff_lines("", "").is_empty());
        assert_eq!(kinds("", "x\n"), vec![(DiffKind::Added, 1)]);
        assert_eq!(kinds("x\n", ""), vec![(DiffKind::Removed, 1)]);
    }

    #[test]
    fn the_diff_string_matches_the_typescript_layout() {
        let old = (1..=12)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let new = old.replace("line 7", "line seven");
        let (diff, first) = generate_diff_string(&old, &new, 4);
        assert_eq!(first, Some(7));
        assert_eq!(
            diff,
            "    ...\n  3 line 3\n  4 line 4\n  5 line 5\n  6 line 6\n- 7 line 7\n+ 7 line seven\n  8 line 8\n  9 line 9\n 10 line 10\n 11 line 11\n    ..."
        );
    }

    #[test]
    fn the_diff_string_of_a_fresh_file_is_all_additions() {
        let (diff, first) = generate_diff_string("", "a\nb\n", 4);
        assert_eq!(diff, "+1 a\n+2 b");
        assert_eq!(first, Some(1));
        let (same, none) = generate_diff_string("a\n", "a\n", 4);
        assert_eq!(same, "");
        assert_eq!(none, None);
    }

    #[test]
    fn applies_multiple_disjoint_edits() {
        let result = apply_edits_to_normalized_content(
            "alpha\nbeta\ngamma\n",
            &[
                Edit {
                    old_text: "alpha".into(),
                    new_text: "ALPHA".into(),
                },
                Edit {
                    old_text: "gamma".into(),
                    new_text: "GAMMA".into(),
                },
            ],
            "x.rs",
        )
        .unwrap();
        assert_eq!(result.new_content, "ALPHA\nbeta\nGAMMA\n");
    }

    #[test]
    fn rejects_duplicate_and_overlap() {
        let duplicate = apply_edits_to_normalized_content(
            "foo foo",
            &[Edit {
                old_text: "foo".into(),
                new_text: "bar".into(),
            }],
            "a.txt",
        )
        .unwrap_err();
        assert!(duplicate.contains("Found 2 occurrences of the text in a.txt"));
        let overlap = apply_edits_to_normalized_content(
            "abcdef",
            &[
                Edit {
                    old_text: "abc".into(),
                    new_text: "xxx".into(),
                },
                Edit {
                    old_text: "cde".into(),
                    new_text: "yyy".into(),
                },
            ],
            "b.txt",
        )
        .unwrap_err();
        assert!(overlap.contains("overlap in b.txt"));
    }

    #[test]
    fn parses_json_string_edits_and_legacy_fields() {
        let (_path, edits) = prepare_edit_arguments(&serde_json::json!({
            "path": "a.txt",
            "edits": "[{\"oldText\":\"a\",\"newText\":\"b\"}]"
        }))
        .unwrap();
        assert_eq!(edits.len(), 1);
        let (_path, edits) = prepare_edit_arguments(&serde_json::json!({
            "path": "a.txt",
            "oldText": "a",
            "newText": "b"
        }))
        .unwrap();
        assert_eq!(edits.len(), 1);
    }
}

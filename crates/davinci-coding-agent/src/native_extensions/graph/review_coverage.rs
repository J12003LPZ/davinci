//! Complete review coverage tracking and deterministic patch chunking.
//!
//! Large diffs cannot safely fit within a single reviewer worker's context
//! window without risking silent truncation. This module deterministically
//! chunks mutations along file and line boundaries, assigns stable chunk IDs,
//! and tracks coverage so that an approval verdict is rejected whenever
//! any required chunk was not reviewed.

use super::mutation::GraphMutation;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChunk {
    pub id: String,
    pub file: String,
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCoverage {
    pub required_chunk_ids: Vec<String>,
    pub reviewed_chunk_ids: Vec<String>,
}

impl ReviewCoverage {
    pub fn new(required_chunk_ids: Vec<String>) -> Self {
        Self {
            required_chunk_ids,
            reviewed_chunk_ids: Vec::new(),
        }
    }

    pub fn record_reviewed(&mut self, chunk_ids: &[String]) {
        for id in chunk_ids {
            if !self.reviewed_chunk_ids.contains(id) {
                self.reviewed_chunk_ids.push(id.clone());
            }
        }
    }

    pub fn missing_chunk_ids(&self) -> Vec<String> {
        let reviewed: HashSet<&str> = self.reviewed_chunk_ids.iter().map(String::as_str).collect();
        self.required_chunk_ids
            .iter()
            .filter(|req| !reviewed.contains(req.as_str()))
            .cloned()
            .collect()
    }
}

/// Split a unified diff into review chunks before it reaches a model.
///
/// A `diff --git` header starts a new section. Sections are packed into
/// chunks no larger than `max_bytes`; a single line longer than the budget is
/// split across consecutive chunks, so no part of the diff is dropped. Callers
/// must present these chunk patches verbatim: a truncation marker cannot be
/// evidence that the omitted bytes were reviewed.
pub fn chunk_text_diff(label: &str, diff: &str, max_bytes: usize) -> Vec<ReviewChunk> {
    let max_bytes = max_bytes.max(1);
    if diff.is_empty() {
        return vec![ReviewChunk {
            id: format!("{label}#chunk-0"),
            file: label.to_string(),
            patch: String::new(),
        }];
    }

    // Group lines by file section, keeping the original order and bytes.
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    for line in diff.lines() {
        let starts_section = line.starts_with("diff --git ");
        if starts_section || sections.is_empty() {
            let file = if starts_section {
                line.split_whitespace()
                    .nth(3)
                    .map(|path| path.trim_start_matches("b/"))
                    .filter(|path| !path.is_empty())
                    .unwrap_or(label)
            } else {
                label
            };
            sections.push((file.to_string(), Vec::new()));
        }
        sections
            .last_mut()
            .expect("a section was just created")
            .1
            .push(line);
    }

    let mut chunks = Vec::new();
    let mut index = 0usize;
    for (file, lines) in sections {
        let mut current = String::new();
        for line in lines {
            let mut remaining = line;
            loop {
                let separator = usize::from(!current.is_empty());
                let available = max_bytes.saturating_sub(current.len() + separator);
                if available == 0 {
                    chunks.push(ReviewChunk {
                        id: format!("{file}#chunk-{index}"),
                        file: file.clone(),
                        patch: std::mem::take(&mut current),
                    });
                    index += 1;
                    continue;
                }
                if remaining.len() <= available {
                    if !current.is_empty() {
                        current.push('\n');
                    }
                    current.push_str(remaining);
                    break;
                }
                let split = floor_char_boundary(remaining, available);
                if split == 0 {
                    // `available` cannot hold one character; flush and retry.
                    chunks.push(ReviewChunk {
                        id: format!("{file}#chunk-{index}"),
                        file: file.clone(),
                        patch: std::mem::take(&mut current),
                    });
                    index += 1;
                    continue;
                }
                if !current.is_empty() {
                    current.push('\n');
                }
                current.push_str(&remaining[..split]);
                remaining = &remaining[split..];
                chunks.push(ReviewChunk {
                    id: format!("{file}#chunk-{index}"),
                    file: file.clone(),
                    patch: std::mem::take(&mut current),
                });
                index += 1;
            }
        }
        if !current.is_empty() {
            chunks.push(ReviewChunk {
                id: format!("{file}#chunk-{index}"),
                file: file.clone(),
                patch: current,
            });
            index += 1;
        }
    }
    chunks
}

fn floor_char_boundary(value: &str, max: usize) -> usize {
    if max >= value.len() {
        return value.len();
    }
    let mut boundary = max;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Returns true if every required chunk in the coverage has been reviewed.
pub fn coverage_complete(coverage: &ReviewCoverage) -> bool {
    let reviewed: HashSet<&str> = coverage
        .reviewed_chunk_ids
        .iter()
        .map(String::as_str)
        .collect();
    coverage
        .required_chunk_ids
        .iter()
        .all(|required| reviewed.contains(required.as_str()))
}

/// Deterministically chunks a `GraphMutation` into `ReviewChunk`s where no chunk
/// exceeds `max_bytes` unless an individual line itself exceeds `max_bytes`.
///
/// Stable chunk IDs are formatted as `<file>#chunk-<index>`.
pub fn chunk_graph_mutation(mutation: &GraphMutation, max_bytes: usize) -> Vec<ReviewChunk> {
    let max_bytes = if max_bytes == 0 { 1 } else { max_bytes };
    let mut chunks = Vec::new();

    for patch_chunk in &mutation.patch_chunks {
        let file = &patch_chunk.file;
        let patch = &patch_chunk.patch;

        if patch.is_empty() {
            chunks.push(ReviewChunk {
                id: format!("{file}#chunk-0"),
                file: file.clone(),
                patch: String::new(),
            });
            continue;
        }

        if patch.len() <= max_bytes {
            chunks.push(ReviewChunk {
                id: format!("{file}#chunk-0"),
                file: file.clone(),
                patch: patch.clone(),
            });
            continue;
        }

        let mut current_lines: Vec<&str> = Vec::new();
        let mut current_bytes = 0;
        let mut chunk_idx = 0;

        for line in patch.lines() {
            let line_len = line.len() + 1; // including '\n'
            if !current_lines.is_empty() && current_bytes + line_len > max_bytes {
                chunks.push(ReviewChunk {
                    id: format!("{file}#chunk-{chunk_idx}"),
                    file: file.clone(),
                    patch: current_lines.join("\n"),
                });
                chunk_idx += 1;
                current_lines.clear();
                current_bytes = 0;
            }
            current_lines.push(line);
            current_bytes += line_len;
        }

        if !current_lines.is_empty() {
            chunks.push(ReviewChunk {
                id: format!("{file}#chunk-{chunk_idx}"),
                file: file.clone(),
                patch: current_lines.join("\n"),
            });
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::mutation::{ChangedFile, PatchChunk};

    #[test]
    fn test_empty_mutation_produces_no_chunks_and_coverage_is_complete() {
        let mutation = GraphMutation::default();
        let chunks = chunk_graph_mutation(&mutation, 1000);
        assert!(chunks.is_empty());

        let coverage = ReviewCoverage::new(vec![]);
        assert!(coverage_complete(&coverage));
        assert!(coverage.missing_chunk_ids().is_empty());
    }

    #[test]
    fn test_small_mutation_produces_single_chunk_per_file() {
        let mutation = GraphMutation {
            files: vec![ChangedFile::modified("src/lib.rs")],
            patch_chunks: vec![PatchChunk {
                file: "src/lib.rs".into(),
                patch: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new".into(),
            }],
        };

        let chunks = chunk_graph_mutation(&mutation, 1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, "src/lib.rs#chunk-0");
        assert_eq!(chunks[0].file, "src/lib.rs");
        assert!(chunks[0].patch.contains("+new"));

        let mut coverage = ReviewCoverage::new(vec![chunks[0].id.clone()]);
        assert!(!coverage_complete(&coverage));
        coverage.record_reviewed(&[chunks[0].id.clone()]);
        assert!(coverage_complete(&coverage));
    }

    #[test]
    fn test_large_diff_fixture_exceeding_max_bytes_is_chunked_deterministically() {
        let mut patch_lines = vec![
            "--- a/src/large.rs".to_string(),
            "+++ b/src/large.rs".to_string(),
            "@@ -1,50 +1,50 @@".to_string(),
        ];
        for i in 0..50 {
            patch_lines.push(format!("+line_{i:03} = \"some value for line {i}\";"));
        }
        let full_patch = patch_lines.join("\n");

        let mutation = GraphMutation {
            files: vec![ChangedFile::modified("src/large.rs")],
            patch_chunks: vec![PatchChunk {
                file: "src/large.rs".into(),
                patch: full_patch.clone(),
            }],
        };

        // Chunk with max_bytes = 200 (much smaller than ~2KB diff)
        let chunks = chunk_graph_mutation(&mutation, 200);
        assert!(chunks.len() > 5, "Expected multiple chunks for large diff");

        // Verify chunk IDs are deterministic and stable
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.id, format!("src/large.rs#chunk-{i}"));
            assert_eq!(chunk.file, "src/large.rs");
            assert!(!chunk.patch.is_empty());
        }

        // Verify all lines are preserved across chunks
        let reassembled: String = chunks
            .iter()
            .map(|c| c.patch.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(reassembled, full_patch);
    }

    #[test]
    fn test_approval_impossible_when_one_chunk_is_omitted() {
        let required = vec![
            "src/a.rs#chunk-0".to_string(),
            "src/a.rs#chunk-1".to_string(),
            "src/b.rs#chunk-0".to_string(),
        ];

        // Worker only reviewed chunks 0 of a.rs and chunk 0 of b.rs (omitted a.rs#chunk-1)
        let mut coverage = ReviewCoverage::new(required.clone());
        coverage.record_reviewed(&[
            "src/a.rs#chunk-0".to_string(),
            "src/b.rs#chunk-0".to_string(),
        ]);

        assert!(
            !coverage_complete(&coverage),
            "Coverage must NOT be complete when a chunk is omitted"
        );
        let missing = coverage.missing_chunk_ids();
        assert_eq!(missing, vec!["src/a.rs#chunk-1".to_string()]);

        // When missing chunk is finally reviewed:
        coverage.record_reviewed(&["src/a.rs#chunk-1".to_string()]);
        assert!(coverage_complete(&coverage));
        assert!(coverage.missing_chunk_ids().is_empty());
    }

    #[test]
    fn text_diff_is_chunked_without_dropping_any_bytes() {
        let mut diff = String::new();
        for file in ["src/a.rs", "src/b.rs"] {
            diff.push_str(&format!("diff --git a/{file} b/{file}\n"));
            diff.push_str(&format!("--- a/{file}\n+++ b/{file}\n"));
            for line in 0..120 {
                diff.push_str(&format!(
                    "+{file} line {line} with enough text to need several chunks\n"
                ));
            }
        }

        let chunks = chunk_text_diff("workspace", &diff, 512);
        assert!(chunks.len() > 5, "expected many chunks: {}", chunks.len());
        assert!(chunks.iter().all(|chunk| !chunk.patch.is_empty()));
        let ids: HashSet<&str> = chunks.iter().map(|chunk| chunk.id.as_str()).collect();
        assert_eq!(ids.len(), chunks.len(), "chunk IDs must be unique");
        assert!(chunks.iter().any(|chunk| chunk.file == "src/a.rs"));
        assert!(chunks.iter().any(|chunk| chunk.file == "src/b.rs"));

        // Every non-newline byte of the diff appears, in order, across chunks.
        let reassembled: String = chunks.iter().map(|chunk| chunk.patch.as_str()).collect();
        assert_eq!(
            reassembled.replace('\n', ""),
            diff.replace('\n', ""),
            "chunking must not omit or reorder diff content"
        );
    }

    #[test]
    fn a_single_oversized_line_is_split_across_chunks() {
        let line = format!("+{}", "x".repeat(5_000));
        let diff = format!("diff --git a/big.rs b/big.rs\n{line}\n");
        let chunks = chunk_text_diff("workspace", &diff, 256);
        assert!(chunks.len() > 1);
        let reassembled: String = chunks.iter().map(|chunk| chunk.patch.as_str()).collect();
        assert_eq!(reassembled.replace('\n', ""), diff.replace('\n', ""));
    }

    #[test]
    fn an_empty_working_tree_diff_is_still_a_required_chunk() {
        let chunks = chunk_text_diff("workspace", "", 512);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, "workspace#chunk-0");
        assert!(chunks[0].patch.is_empty());
        let coverage = ReviewCoverage::new(vec![chunks[0].id.clone()]);
        assert!(!coverage_complete(&coverage));
    }
}

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
}

use super::super::{repo_intelligence::RepoIndex, workspace_metadata::WorkspaceMetadata};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_EVIDENCE_BYTES: usize = 4 * 1024 * 1024;
#[derive(Default)]
struct Selection {
    rows: BTreeMap<String, Vec<Reason>>,
    bytes: usize,
    limited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reason {
    pub kind: String,
    pub source: String,
    pub chain: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedTest {
    pub path: String,
    pub package: String,
    pub reasons: Vec<Reason>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestMapping {
    pub reverse: BTreeMap<String, BTreeSet<String>>,
    pub candidate_edges: BTreeSet<(String, String)>,
    pub tests: BTreeSet<String>,
    pub warnings: Vec<String>,
}

pub fn is_test(path: &str) -> bool {
    path.contains(".test.")
        || path.contains(".spec.")
        || path
            .split('/')
            .any(|part| matches!(part, "__tests__" | "tests" | "test"))
}

impl TestMapping {
    pub fn from_index(index: &RepoIndex) -> Self {
        let mut value = Self {
            tests: index.files.keys().filter(|p| is_test(p)).cloned().collect(),
            warnings: index.warnings.clone(),
            ..Default::default()
        };
        for edge in index.dependencies() {
            if let Some(target) = edge.to {
                value.reverse.entry(target).or_default().insert(edge.from);
            } else if !edge.external {
                value
                    .warnings
                    .push(format!("unresolved import in {}", edge.from));
                for candidate in index.missing_dependency_candidates(&edge.from, &edge.specifier) {
                    value
                        .candidate_edges
                        .insert((candidate.clone(), edge.from.clone()));
                    value
                        .reverse
                        .entry(candidate)
                        .or_default()
                        .insert(edge.from.clone());
                }
            }
        }
        for file in index.files.values() {
            if file.parse_status != "ok"
                || file
                    .unresolved
                    .iter()
                    .any(|reason| reason.starts_with("dynamic import/require:"))
            {
                value
                    .warnings
                    .push(format!("incomplete source analysis: {}", file.path));
            }
        }
        value.warnings.sort();
        value.warnings.dedup();
        value.warnings.truncate(32);
        value
    }

    pub fn select(
        &self,
        changed: &[String],
        metadata: &WorkspaceMetadata,
    ) -> (Vec<RelatedTest>, Vec<String>) {
        let mut selected = Selection::default();
        let mut warnings = self.warnings.clone();
        'origins: for origin in changed {
            let mut queue = VecDeque::from([vec![origin.clone()]]);
            let mut queue_bytes = origin.len();
            let mut seen = BTreeSet::from([origin.clone()]);
            while let Some(chain) = queue.pop_front() {
                queue_bytes -= chain.iter().map(String::len).sum::<usize>();
                let file = chain.last().unwrap();
                if self.tests.contains(file) {
                    let tentative = chain.windows(2).any(|pair| {
                        self.candidate_edges
                            .contains(&(pair[0].clone(), pair[1].clone()))
                    });
                    add(
                        &mut selected,
                        file,
                        if tentative {
                            "potential_dependency"
                        } else {
                            "dependency"
                        },
                        if tentative {
                            "unresolved-import-candidate"
                        } else {
                            "ast"
                        },
                        chain.clone(),
                    );
                    if selected.limited {
                        break 'origins;
                    }
                }
                if chain.len() > 32 {
                    warnings.push(
                        "dependency traversal depth limit; broader verification required".into(),
                    );
                    continue;
                }
                for next in self.reverse.get(file).into_iter().flatten() {
                    if seen.insert(next.clone()) {
                        let mut extended = chain.clone();
                        extended.push(next.clone());
                        queue_bytes += extended.iter().map(String::len).sum::<usize>();
                        if queue_bytes > MAX_EVIDENCE_BYTES {
                            selected.limited = true;
                            break 'origins;
                        }
                        queue.push_back(extended);
                    }
                }
            }
            let owner = metadata
                .owner(origin)
                .map(|p| p.path.as_str())
                .unwrap_or(".");
            let config = super::super::repo_intelligence::config_file(origin);
            for test in &self.tests {
                let test_owner = metadata.owner(test).map(|p| p.path.as_str()).unwrap_or(".");
                if config && (owner == "." || owner == test_owner) {
                    add(
                        &mut selected,
                        test,
                        "configuration",
                        "config",
                        vec![origin.clone(), test.clone()],
                    );
                } else if owner == test_owner && stem(origin) == stem(test) && test != origin {
                    add(
                        &mut selected,
                        test,
                        "paired_test",
                        "test-map",
                        vec![origin.clone(), test.clone()],
                    );
                }
                if selected.limited {
                    break 'origins;
                }
            }
        }
        if selected.limited {
            warnings.push("dependency evidence limit; selection is incomplete and workspace verification is required".into());
        }
        let rows = selected
            .rows
            .into_iter()
            .map(|(path, reasons)| RelatedTest {
                package: metadata
                    .owner(&path)
                    .map(|p| p.path.clone())
                    .unwrap_or_else(|| ".".into()),
                path,
                reasons,
            })
            .collect();
        warnings.sort();
        warnings.dedup();
        warnings.truncate(32);
        (rows, warnings)
    }
}

fn stem(path: &str) -> &str {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    stem.strip_suffix(".test")
        .or_else(|| stem.strip_suffix(".spec"))
        .unwrap_or(stem)
}

fn add(selection: &mut Selection, path: &str, kind: &str, source: &str, chain: Vec<String>) {
    let bytes = chain.iter().map(String::len).sum::<usize>() + path.len() + 256;
    if selection.bytes + bytes > MAX_EVIDENCE_BYTES {
        selection.limited = true;
        return;
    }
    let reasons = selection.rows.entry(path.to_string()).or_default();
    if reasons.len() < 8 {
        selection.bytes += bytes;
        reasons.push(Reason {
            kind: kind.into(),
            source: source.into(),
            chain,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impact_caps_evidence_without_emitting_unexplained_rows() {
        let mut selection = Selection {
            bytes: MAX_EVIDENCE_BYTES - 16,
            ..Default::default()
        };
        add(
            &mut selection,
            "test.ts",
            "dependency",
            "ast",
            vec!["source.ts".into(), "test.ts".into()],
        );
        assert!(selection.limited);
        assert!(selection.rows.is_empty());
    }
}

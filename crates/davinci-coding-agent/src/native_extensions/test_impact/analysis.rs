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
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.contains(".test.") || name.contains(".spec.") {
        return true;
    }
    // Files merely stored under a test directory are tests only when they are
    // not fixtures, helpers, mocks, setup files or type declarations.
    let mut dirs = path.split('/').collect::<Vec<_>>();
    dirs.pop();
    let in_test_dir = dirs
        .iter()
        .any(|part| matches!(*part, "__tests__" | "tests" | "test"));
    let support_dir = dirs.iter().any(|part| {
        matches!(
            *part,
            "fixtures"
                | "fixture"
                | "__fixtures__"
                | "helpers"
                | "helper"
                | "support"
                | "mocks"
                | "__mocks__"
                | "utils"
                | "snapshots"
                | "__snapshots__"
                | "setup"
        )
    });
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let support_file = name.ends_with(".d.ts")
        || matches!(
            stem,
            "setup" | "helper" | "helpers" | "utils" | "util" | "mock" | "mocks" | "fixture"
        )
        || stem.starts_with("setup")
        || stem.ends_with("-helper")
        || stem.ends_with("_helper");
    in_test_dir && !support_dir && !support_file
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
                } else if owner == test_owner
                    && pair_key(origin) == pair_key(test)
                    && test != origin
                {
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

/// Identity of the module a source or test file is about: its directory
/// (ignoring `src` / `test` containers) plus its stem, so `src/auth/config.ts`
/// pairs with `tests/auth/config.test.ts` but not with `src/payments/config.ts`.
fn pair_key(path: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.retain(|part| {
        !matches!(
            *part,
            "src" | "lib" | "test" | "tests" | "__tests__" | "spec"
        )
    });
    (parts, stem(path))
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
    fn helpers_and_fixtures_under_test_directories_are_not_tests() {
        for path in [
            "tests/fixtures/helper.ts",
            "test/setup.ts",
            "tests/helpers/db.ts",
            "src/__tests__/__mocks__/api.ts",
            "tests/types.d.ts",
        ] {
            assert!(!is_test(path), "{path}");
        }
        for path in [
            "src/a.test.ts",
            "src/b.spec.js",
            "tests/auth/config.ts",
            "src/__tests__/login.ts",
            "tests/fixtures/case.test.ts",
        ] {
            assert!(is_test(path), "{path}");
        }
    }

    #[test]
    fn pairing_requires_the_same_module_directory() {
        let source = "src/payments/config.ts";
        assert_ne!(pair_key(source), pair_key("tests/auth/config.test.ts"));
        assert_eq!(
            pair_key("src/auth/config.ts"),
            pair_key("tests/auth/config.test.ts")
        );
        assert_eq!(
            pair_key("src/auth/config.ts"),
            pair_key("src/auth/config.test.ts")
        );
    }

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

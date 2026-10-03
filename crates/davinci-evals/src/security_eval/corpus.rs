//! Held-out corpus structure and label-free worker inputs. Native-only contract.
//! Structural validation is not independent adjudication or measured model quality.

use super::CausalIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub type SourceCases = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    AuthIdor,
    TenantIsolation,
    InjectionXss,
    Filesystem,
    SecretsConfiguration,
    DependencyApplicability,
    CryptoSession,
    RaceBusinessLogic,
    SubprocessApproval,
    PromptToolPlugin,
}

pub const FAMILIES: [Family; 10] = [
    Family::AuthIdor,
    Family::TenantIsolation,
    Family::InjectionXss,
    Family::Filesystem,
    Family::SecretsConfiguration,
    Family::DependencyApplicability,
    Family::CryptoSession,
    Family::RaceBusinessLogic,
    Family::SubprocessApproval,
    Family::PromptToolPlugin,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Informational,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Confirmed,
    Likely,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceRole {
    Entrypoint,
    Source,
    Control,
    Sink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Anchor {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub role: EvidenceRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub id: String,
    /// Exact relative source allowlist and SHA-256 digests; no directory discovery.
    pub files: BTreeMap<String, String>,
    pub decisive_evidence: Vec<Anchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedFinding {
    pub identity: CausalIdentity,
    pub actor: String,
    pub preconditions: Vec<String>,
    pub classification: Classification,
    pub minimum_severity: Severity,
    pub maximum_severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pair {
    pub id: String,
    pub family: Family,
    pub vulnerable: Case,
    pub secure: Case,
    pub expected: ExpectedFinding,
    pub secure_control_rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Corpus {
    pub schema_version: u32,
    pub id: String,
    pub pairs: Vec<Pair>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSummary {
    pub pairs: usize,
    pub cases: usize,
    pub pairs_by_family: BTreeMap<Family, usize>,
    pub cross_file_vulnerable_cases: usize,
}

/// Only this validated view exposes source inputs; labels remain evaluator-owned.
#[derive(Debug)]
pub struct ValidatedCorpus<'a> {
    sources: &'a SourceCases,
    summary: CorpusSummary,
}

impl ValidatedCorpus<'_> {
    pub fn summary(&self) -> &CorpusSummary {
        &self.summary
    }

    /// This projection contains no case IDs, family, expected findings or rationale.
    /// A runner must stage only these files in its isolated scan directory.
    pub fn worker_files(&self, case_id: &str) -> Option<&BTreeMap<String, String>> {
        self.sources.get(case_id)
    }
}

impl Corpus {
    pub fn validate<'a>(&self, sources: &'a SourceCases) -> Result<ValidatedCorpus<'a>, String> {
        identifier(&self.id)?;
        if self.schema_version != 1 || !(40..=1000).contains(&self.pairs.len()) {
            return Err("corpus requires schema 1 and 40..1000 paired cases".into());
        }
        let mut pair_ids = BTreeSet::new();
        let mut case_ids = BTreeSet::new();
        let mut counts: BTreeMap<_, _> = FAMILIES.into_iter().map(|family| (family, 0)).collect();
        let mut cross_file = 0;
        let mut bytes = 0u64;
        for pair in &self.pairs {
            identifier(&pair.id)?;
            if !pair_ids.insert(&pair.id) {
                return Err("duplicate corpus pair ID".into());
            }
            validate_expected(&pair.expected)?;
            text(&pair.secure_control_rationale)?;
            let vulnerable = validate_case(&pair.vulnerable, sources, &mut case_ids, &mut bytes)?;
            validate_case(&pair.secure, sources, &mut case_ids, &mut bytes)?;
            if !pair
                .vulnerable
                .decisive_evidence
                .iter()
                .any(|anchor| anchor.role == EvidenceRole::Sink)
            {
                return Err("vulnerable case requires decisive sink evidence".into());
            }
            cross_file += usize::from(vulnerable.len() >= 2);
            *counts
                .get_mut(&pair.family)
                .ok_or("unknown corpus family")? += 1;
        }
        if sources.len() != case_ids.len() || sources.keys().any(|id| !case_ids.contains(id)) {
            return Err("source cases differ from the evaluator allowlist".into());
        }
        if counts.values().any(|count| *count < 4) || cross_file < 10 {
            return Err(
                "corpus requires four pairs per family and ten cross-file vulnerable cases".into(),
            );
        }
        Ok(ValidatedCorpus {
            sources,
            summary: CorpusSummary {
                pairs: self.pairs.len(),
                cases: case_ids.len(),
                pairs_by_family: counts,
                cross_file_vulnerable_cases: cross_file,
            },
        })
    }
}

fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(
            "corpus IDs require 1..128 ASCII alphanumeric, underscore or hyphen characters".into(),
        );
    }
    Ok(())
}

fn text(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 8192 || value.contains('\0') {
        return Err("corpus ground-truth text is empty or exceeds its bound".into());
    }
    Ok(())
}

fn validate_expected(expected: &ExpectedFinding) -> Result<(), String> {
    for value in [
        &expected.identity.control,
        &expected.identity.trust_boundary,
        &expected.identity.sink,
        &expected.identity.occurrence,
        &expected.actor,
    ] {
        text(value)?;
    }
    if expected.preconditions.len() > 32 || expected.minimum_severity > expected.maximum_severity {
        return Err("invalid ground-truth preconditions or severity band".into());
    }
    expected
        .preconditions
        .iter()
        .try_for_each(|value| text(value))
}

const MAX_SOURCE_PATH: usize = 1024;
const MAX_SOURCE_COMPONENT: usize = 255;

pub(super) fn source_path(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > MAX_SOURCE_PATH
        || value.starts_with('/')
        || value.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || value.chars().any(char::is_control)
    {
        return Err("invalid corpus source path".into());
    }
    for component in value.split('/') {
        let lower = component.to_ascii_lowercase();
        let stem = lower.split('.').next().unwrap_or("");
        if component.is_empty()
            || component.len() > MAX_SOURCE_COMPONENT
            || [".", "..", ".git"].contains(&lower.as_str())
            || component.ends_with(['.', ' '])
            || ["con", "prn", "aux", "nul"].contains(&stem)
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("corpus source path is not portable and repository-relative".into());
        }
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_case<'a>(
    case: &'a Case,
    sources: &SourceCases,
    seen: &mut BTreeSet<&'a String>,
    bytes: &mut u64,
) -> Result<BTreeSet<String>, String> {
    identifier(&case.id)?;
    if !seen.insert(&case.id) {
        return Err("duplicate corpus case ID".into());
    }
    let files = sources
        .get(&case.id)
        .ok_or("corpus case source is missing")?;
    if !(1..=128).contains(&files.len()) || !case.files.keys().eq(files.keys()) {
        return Err("case source files differ from the exact source allowlist".into());
    }
    let mut paths = BTreeSet::new();
    let mut case_bytes = 0u64;
    for (path, source) in files {
        if !paths.insert(source_path(path)?) {
            return Err("case source paths collide".into());
        }
        case_bytes = case_bytes
            .checked_add(source.len() as u64)
            .ok_or("case byte count overflow")?;
        if case_bytes > 16 * 1024 * 1024 || source.contains('\0') {
            return Err("corpus case exceeds text source limits".into());
        }
        if case.files[path] != format!("{:x}", Sha256::digest(source.as_bytes())) {
            return Err("corpus source digest mismatch".into());
        }
    }
    for path in &paths {
        for (index, _) in path.match_indices('/') {
            if paths.contains(&path[..index]) {
                return Err("corpus file collides with a source directory".into());
            }
        }
    }
    *bytes = bytes
        .checked_add(case_bytes)
        .ok_or("corpus byte count overflow")?;
    if *bytes > 512 * 1024 * 1024 {
        return Err("corpus exceeds aggregate source limit".into());
    }
    if !(1..=64).contains(&case.decisive_evidence.len())
        || !case
            .decisive_evidence
            .iter()
            .any(|anchor| anchor.role == EvidenceRole::Control)
    {
        return Err("case requires bounded decisive control evidence".into());
    }
    let mut decisive_paths = BTreeSet::new();
    for anchor in &case.decisive_evidence {
        let source = files
            .get(&anchor.path)
            .ok_or("decisive evidence references unlisted source")?;
        if anchor.start_line == 0
            || anchor.end_line < anchor.start_line
            || anchor.end_line - anchor.start_line >= 200
            || anchor.end_line > source.lines().count()
        {
            return Err("decisive evidence range is outside captured source".into());
        }
        decisive_paths.insert(anchor.path.clone());
    }
    Ok(decisive_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture() -> (Corpus, SourceCases) {
        let mut sources = SourceCases::new();
        let mut pairs = Vec::new();
        for (family_index, family) in FAMILIES.iter().enumerate() {
            for index in 0..4 {
                let pair_id = format!("pair-{family_index}-{index}");
                let mut case = |suffix: &str| {
                    let id = format!("{pair_id}-{suffix}");
                    let files: BTreeMap<_, _> = [
                        ("control.rs".into(), "fn control() {}\n".into()),
                        ("sink.rs".into(), "fn sink() {}\n".into()),
                    ]
                    .into();
                    let hashes = files
                        .iter()
                        .map(|(path, text): (&String, &String)| {
                            (
                                path.clone(),
                                format!("{:x}", Sha256::digest(text.as_bytes())),
                            )
                        })
                        .collect();
                    sources.insert(id.clone(), files);
                    Case {
                        id,
                        files: hashes,
                        decisive_evidence: vec![
                            Anchor {
                                path: "control.rs".into(),
                                start_line: 1,
                                end_line: 1,
                                role: EvidenceRole::Control,
                            },
                            Anchor {
                                path: "sink.rs".into(),
                                start_line: 1,
                                end_line: 1,
                                role: EvidenceRole::Sink,
                            },
                        ],
                    }
                };
                pairs.push(Pair {
                    id: pair_id.clone(),
                    family: *family,
                    vulnerable: case("a"),
                    secure: case("b"),
                    expected: ExpectedFinding {
                        identity: CausalIdentity {
                            control: "evaluator-only-control-label".into(),
                            trust_boundary: "fixture boundary".into(),
                            sink: "fixture sink".into(),
                            occurrence: "fixture occurrence".into(),
                        },
                        actor: "fixture actor".into(),
                        preconditions: vec![],
                        classification: Classification::Confirmed,
                        minimum_severity: Severity::Medium,
                        maximum_severity: Severity::High,
                    },
                    secure_control_rationale: "evaluator-only-secure-rationale".into(),
                });
            }
        }
        (
            Corpus {
                schema_version: 1,
                id: "synthetic-contract-fixture".into(),
                pairs,
            },
            sources,
        )
    }

    #[test]
    fn security_corpus_requires_all_pairs_families_and_separate_worker_inputs() {
        let (corpus, sources) = fixture();
        let validated = corpus.validate(&sources).unwrap();
        assert_eq!(validated.summary().pairs, 40);
        assert_eq!(validated.summary().cases, 80);
        assert_eq!(validated.summary().cross_file_vulnerable_cases, 40);
        assert!(validated
            .summary()
            .pairs_by_family
            .values()
            .all(|count| *count == 4));
        let worker = serde_json::to_string(
            validated
                .worker_files(&corpus.pairs[0].vulnerable.id)
                .unwrap(),
        )
        .unwrap();
        assert!(!worker.contains("evaluator-only"));
        assert!(!worker.contains(&corpus.pairs[0].vulnerable.id));
        assert!(validated.worker_files("unknown").is_none());
    }

    #[test]
    fn security_corpus_rejects_missing_family_pair_and_unbound_source() {
        let (corpus, sources) = fixture();
        let mut bad = corpus.clone();
        bad.pairs[0].family = Family::Filesystem;
        assert!(bad.validate(&sources).is_err());
        let mut bad = corpus.clone();
        bad.pairs.pop();
        assert!(bad.validate(&sources).is_err());
        let mut bad = sources.clone();
        bad.get_mut(&corpus.pairs[0].vulnerable.id)
            .unwrap()
            .insert("labels.json".into(), "evaluator-only labels".into());
        assert!(corpus.validate(&bad).is_err());
        let mut bad = sources.clone();
        bad.get_mut(&corpus.pairs[0].vulnerable.id)
            .unwrap()
            .insert("control.rs".into(), "changed\n".into());
        assert!(corpus.validate(&bad).is_err());
    }

    fn retarget_vulnerable_control(corpus: &mut Corpus, sources: &mut SourceCases, path: &str) {
        let id = corpus.pairs[0].vulnerable.id.clone();
        let files = sources.get_mut(&id).unwrap();
        let text = files.remove("control.rs").unwrap();
        files.insert(path.into(), text);
        let digest = corpus.pairs[0]
            .vulnerable
            .files
            .remove("control.rs")
            .unwrap();
        corpus.pairs[0].vulnerable.files.insert(path.into(), digest);
        for anchor in &mut corpus.pairs[0].vulnerable.decisive_evidence {
            if anchor.path == "control.rs" {
                anchor.path = path.into();
            }
        }
    }

    #[test]
    fn security_corpus_rejects_non_portable_and_colliding_source_paths() {
        let reserved = [
            "foo<.rs",
            "foo>.rs",
            "foo:.rs",
            "foo\".rs",
            "foo|.rs",
            "foo?.rs",
            "foo*.rs",
            "foo\\bar.rs",
            "C:/windows.rs",
            "foo.rs:stream",
            "../escape.rs",
            "foo/../escape.rs",
            "foo/./x.rs",
            ".git/config",
            "foo/.git/x.rs",
            "con.rs",
            "PRN.txt",
            "aux.rs",
            "nul.rs",
            "com1.rs",
            "lpt9.rs",
            "foo./x.rs",
            "foo /x.rs",
            "/abs.rs",
            "foo//bar.rs",
        ];
        for path in reserved {
            let (mut corpus, mut sources) = fixture();
            retarget_vulnerable_control(&mut corpus, &mut sources, path);
            assert!(
                corpus.validate(&sources).is_err(),
                "accepted non-portable path {path:?}"
            );
        }

        let (mut corpus, mut sources) = fixture();
        retarget_vulnerable_control(
            &mut corpus,
            &mut sources,
            &format!("{}.rs", "a".repeat(256)),
        );
        assert!(corpus.validate(&sources).is_err());

        let (mut corpus, mut sources) = fixture();
        let id = corpus.pairs[0].vulnerable.id.clone();
        let text = sources[&id]["control.rs"].clone();
        sources
            .get_mut(&id)
            .unwrap()
            .insert("Control.rs".into(), text.clone());
        let digest = corpus.pairs[0].vulnerable.files["control.rs"].clone();
        corpus.pairs[0]
            .vulnerable
            .files
            .insert("Control.rs".into(), digest);
        assert_eq!(
            corpus.validate(&sources).unwrap_err(),
            "case source paths collide"
        );
    }

    #[test]
    fn security_corpus_rejects_insufficient_cross_file_evidence() {
        let (mut corpus, sources) = fixture();
        for pair in corpus.pairs.iter_mut().skip(9) {
            pair.vulnerable
                .decisive_evidence
                .retain(|anchor| anchor.role == EvidenceRole::Control);
            pair.vulnerable.decisive_evidence.push(Anchor {
                path: "control.rs".into(),
                start_line: 1,
                end_line: 1,
                role: EvidenceRole::Sink,
            });
        }
        assert_eq!(
            corpus.validate(&sources).unwrap_err(),
            "corpus requires four pairs per family and ten cross-file vulnerable cases"
        );
    }

    #[test]
    fn security_corpus_rejects_duplicate_ids_and_source_set_mismatch() {
        let (corpus, sources) = fixture();
        let mut bad = corpus.clone();
        bad.pairs[1].id = bad.pairs[0].id.clone();
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "duplicate corpus pair ID"
        );

        let mut bad = corpus.clone();
        bad.pairs[1].vulnerable.id = bad.pairs[0].vulnerable.id.clone();
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "duplicate corpus case ID"
        );

        let mut missing = sources.clone();
        missing.remove(&corpus.pairs[0].vulnerable.id);
        assert_eq!(
            corpus.validate(&missing).unwrap_err(),
            "corpus case source is missing"
        );

        let mut extra = sources.clone();
        extra.insert(
            "extra-case".into(),
            [("control.rs".into(), "fn control() {}\n".into())].into(),
        );
        assert_eq!(
            corpus.validate(&extra).unwrap_err(),
            "source cases differ from the evaluator allowlist"
        );
    }

    #[test]
    fn security_corpus_rejects_invalid_ground_truth_and_anchors() {
        let (corpus, sources) = fixture();
        let mut bad = corpus.clone();
        bad.pairs[0].expected.actor.clear();
        assert!(bad.validate(&sources).is_err());

        let mut bad = corpus.clone();
        bad.pairs[0].expected.minimum_severity = Severity::Critical;
        bad.pairs[0].expected.maximum_severity = Severity::Low;
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "invalid ground-truth preconditions or severity band"
        );

        let mut bad = corpus.clone();
        bad.pairs[0].expected.preconditions = (0..33).map(|i| format!("pre-{i}")).collect();
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "invalid ground-truth preconditions or severity band"
        );

        let mut bad = corpus.clone();
        bad.pairs[0].secure_control_rationale.clear();
        assert!(bad.validate(&sources).is_err());

        let mut bad = corpus.clone();
        bad.pairs[0].vulnerable.decisive_evidence[0].start_line = 0;
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "decisive evidence range is outside captured source"
        );

        let mut bad = corpus.clone();
        bad.pairs[0].vulnerable.decisive_evidence[0].path = "missing.rs".into();
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "decisive evidence references unlisted source"
        );

        let mut bad = corpus.clone();
        bad.pairs[0]
            .vulnerable
            .decisive_evidence
            .retain(|anchor| anchor.role != EvidenceRole::Sink);
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "vulnerable case requires decisive sink evidence"
        );

        let mut bad = corpus.clone();
        bad.pairs[0]
            .secure
            .decisive_evidence
            .retain(|anchor| anchor.role != EvidenceRole::Control);
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "case requires bounded decisive control evidence"
        );
    }

    #[test]
    fn security_corpus_rejects_unknown_schema_fields() {
        let (corpus, _) = fixture();
        let mut value = serde_json::to_value(&corpus).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<Corpus>(value).is_err());

        let mut pair = serde_json::to_value(&corpus.pairs[0]).unwrap();
        pair.as_object_mut()
            .unwrap()
            .insert("labels".into(), serde_json::json!("secret"));
        assert!(serde_json::from_value::<Pair>(pair).is_err());

        let mut case = serde_json::to_value(&corpus.pairs[0].vulnerable).unwrap();
        case.as_object_mut()
            .unwrap()
            .insert("expected".into(), serde_json::json!({}));
        assert!(serde_json::from_value::<Case>(case).is_err());

        let mut bad = corpus.clone();
        bad.schema_version = 2;
        let sources = SourceCases::new();
        assert_eq!(
            bad.validate(&sources).unwrap_err(),
            "corpus requires schema 1 and 40..1000 paired cases"
        );
    }

    #[test]
    fn security_corpus_worker_projection_omits_manifest_metadata_not_source_bytes() {
        let (mut corpus, mut sources) = fixture();
        let id = corpus.pairs[0].vulnerable.id.clone();
        let labeled = "fn control() { /* evaluator-only-control-label */ }\n".to_string();
        sources
            .get_mut(&id)
            .unwrap()
            .insert("control.rs".into(), labeled.clone());
        corpus.pairs[0].vulnerable.files.insert(
            "control.rs".into(),
            format!("{:x}", Sha256::digest(labeled.as_bytes())),
        );
        let validated = corpus.validate(&sources).unwrap();
        let worker = validated.worker_files(&id).unwrap();
        assert_eq!(worker.get("control.rs"), Some(&labeled));
        assert!(worker.contains_key("sink.rs"));
        let projected = serde_json::to_value(worker).unwrap();
        assert!(projected.get("family").is_none());
        assert!(projected.get("expected").is_none());
        assert!(projected.get("secureControlRationale").is_none());
        assert!(
            worker
                .values()
                .any(|source| source.contains("evaluator-only-control-label")),
            "excluding evaluator metadata does not strip labels copied into source bytes"
        );
    }
}

use davinci_agent::runtime::operations::{
    digest_bytes, JournalId, JournalIdentity, LegacyObservation, LegacySourceKind,
    OperationJournal, RootNamespaceId, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::ToolCallLedger;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    journal: OperationJournal,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let journal = OperationJournal::open(
            &temp_root.join("operations"),
            identity,
            RootNamespaceId::new(),
        )
        .unwrap();
        Self {
            _temp: temp,
            journal,
        }
    }
}

#[test]
fn legacy_observations_are_source_bound_and_idempotent() {
    let fixture = Fixture::new();
    let source = br#"{"legacy":"record","state":"running"}"#;
    let mut observation = LegacyObservation::from_source(
        LegacySourceKind::ToolLedger,
        "/workspace/session.tool-ledger.json",
        source,
        "call-1",
        "executing",
    )
    .unwrap();
    observation
        .evidence_provenance
        .push("tool_ledger.status".into());

    let first = fixture
        .journal
        .import_legacy_observations(&[observation.clone()])
        .unwrap();
    let second = fixture
        .journal
        .import_legacy_observations(&[observation.clone()])
        .unwrap();
    assert_eq!((first.imported, first.already_present), (1, 0));
    assert_eq!((second.imported, second.already_present), (0, 1));
    assert_eq!(
        fixture.journal.legacy_observations().unwrap(),
        vec![observation]
    );
}

#[test]
fn legacy_running_observation_never_invents_owner_or_effect_facts() {
    let source = br#"running"#;
    let observation = LegacyObservation::from_source(
        LegacySourceKind::SessionTaskJournal,
        "/workspace/session.runtime.jsonl",
        source,
        "task-1",
        "running",
    )
    .unwrap();
    assert_eq!(observation.source_digest, digest_bytes(source));
    assert_eq!(observation.started_at_ms, None);
    assert_eq!(observation.effect_status, None);
    assert_eq!(observation.owner_id, None);
    assert_eq!(observation.known_result, None);
}

#[test]
fn changed_legacy_record_with_same_source_identity_is_rejected() {
    let fixture = Fixture::new();
    let source = br#"legacy"#;
    let first = LegacyObservation::from_source(
        LegacySourceKind::GraphRun,
        "/workspace/.pi/graph/runs/run-1/state.json",
        source,
        "run-1",
        "failed",
    )
    .unwrap();
    fixture
        .journal
        .import_legacy_observations(&[first.clone()])
        .unwrap();
    let mut changed = first;
    changed.state = "completed".into();
    let error = fixture
        .journal
        .import_legacy_observations(&[changed])
        .unwrap_err();
    assert!(error.to_string().contains("legacy observation changed"));
    assert_eq!(fixture.journal.legacy_observations().unwrap().len(), 1);
}

#[test]
fn corrupt_legacy_tool_ledger_is_read_only_and_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tool-ledger.json");
    std::fs::write(&path, b"{not-json").unwrap();
    let error = ToolCallLedger::legacy_observations_from_path(&path, "session-a").unwrap_err();
    assert!(error.contains("tool ledger is corrupt"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{not-json");
}

#[test]
fn legacy_tool_fixture_is_imported_without_rewriting_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tool-ledger.json");
    let fixture = include_bytes!("fixtures/operations/legacy_tool_ledger.json");
    std::fs::write(&path, fixture).unwrap();

    let observations =
        ToolCallLedger::legacy_observations_from_path(&path, "legacy-session").unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].record_identity, "call-legacy");
    assert_eq!(observations[0].state, "executing");
    assert_eq!(observations[0].source_digest, digest_bytes(fixture));
    assert!(observations[0]
        .evidence_provenance
        .iter()
        .any(|value| value == "tool_ledger.status"));
    assert_eq!(std::fs::read(&path).unwrap(), fixture);
}

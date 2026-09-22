use super::store_api::{JournalError, JournalIdentity};
use super::OperationSpec;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

const INITIAL_SCHEMA_VERSION: u32 = 1;
pub(super) const SCHEMA_VERSION: u32 = 2;
const APPLICATION_ID: i64 = 0x4456_4F50; // "DVOP"

const SCHEMA_V1: &str = r#"
CREATE TABLE journal_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    journal_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    workspace_binding_version INTEGER NOT NULL CHECK (workspace_binding_version > 0),
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE journal_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);
CREATE TABLE journal_roots (
    root_namespace_id TEXT PRIMARY KEY,
    journal_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    workspace_binding_version INTEGER NOT NULL CHECK (workspace_binding_version > 0),
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE operations (
    operation_id TEXT PRIMARY KEY,
    root_namespace_id TEXT NOT NULL REFERENCES journal_roots(root_namespace_id),
    caller_scope TEXT NOT NULL,
    caller_key TEXT NOT NULL CHECK (length(caller_key) > 0),
    payload_digest TEXT NOT NULL CHECK (length(payload_digest) = 64),
    spec_json TEXT NOT NULL CHECK (length(spec_json) <= 1048576),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (root_namespace_id, caller_scope, caller_key)
);
CREATE INDEX operations_root_created ON operations(root_namespace_id, created_at_ms, operation_id);
CREATE TABLE attempts (
    attempt_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    owner_id TEXT NOT NULL,
    owner_generation INTEGER NOT NULL CHECK (owner_generation > 0),
    state TEXT NOT NULL,
    attempt_json TEXT NOT NULL CHECK (length(attempt_json) <= 65536),
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (operation_id, attempt_number)
);
CREATE INDEX attempts_operation_number ON attempts(operation_id, attempt_number);
CREATE TABLE operation_events (
    event_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    root_namespace_id TEXT NOT NULL REFERENCES journal_roots(root_namespace_id),
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    prior_revision INTEGER NOT NULL CHECK (prior_revision >= 0),
    revision INTEGER NOT NULL CHECK (revision = prior_revision + 1),
    event_json TEXT NOT NULL CHECK (length(event_json) <= 65536),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (attempt_id, revision)
);
CREATE INDEX operation_events_root_sequence ON operation_events(root_namespace_id, event_sequence);
CREATE TABLE operation_results (
    result_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    attempt_id TEXT NOT NULL UNIQUE REFERENCES attempts(attempt_id),
    payload_digest TEXT NOT NULL CHECK (length(payload_digest) = 64),
    result_ref_json TEXT NOT NULL CHECK (length(result_ref_json) <= 65536),
    payload_json TEXT NOT NULL CHECK (length(payload_json) <= 1048576),
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE operation_outbox (
    outbox_id TEXT PRIMARY KEY,
    root_namespace_id TEXT NOT NULL REFERENCES journal_roots(root_namespace_id),
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    event_sequence INTEGER NOT NULL REFERENCES operation_events(event_sequence),
    consumer TEXT NOT NULL CHECK (length(consumer) > 0 AND length(consumer) <= 128),
    payload_json TEXT NOT NULL CHECK (length(payload_json) <= 262144),
    state TEXT NOT NULL CHECK (state IN ('pending', 'acknowledged')),
    created_at_ms INTEGER NOT NULL,
    acknowledged_at_ms INTEGER,
    CHECK ((state = 'pending' AND acknowledged_at_ms IS NULL)
        OR (state = 'acknowledged' AND acknowledged_at_ms IS NOT NULL))
);
CREATE INDEX operation_outbox_pending ON operation_outbox(root_namespace_id, state, created_at_ms, outbox_id);
CREATE TRIGGER operations_immutable_update BEFORE UPDATE ON operations
BEGIN SELECT RAISE(ABORT, 'operation specifications are immutable'); END;
CREATE TRIGGER operations_immutable_delete BEFORE DELETE ON operations
BEGIN SELECT RAISE(ABORT, 'operation specifications are immutable'); END;
CREATE TRIGGER journal_metadata_immutable_update BEFORE UPDATE ON journal_metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;
CREATE TRIGGER journal_metadata_immutable_delete BEFORE DELETE ON journal_metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;
CREATE TRIGGER roots_immutable_update BEFORE UPDATE ON journal_roots
BEGIN SELECT RAISE(ABORT, 'root bindings are immutable'); END;
CREATE TRIGGER roots_immutable_delete BEFORE DELETE ON journal_roots
BEGIN SELECT RAISE(ABORT, 'root bindings are immutable'); END;
CREATE TRIGGER attempts_revision_guard BEFORE UPDATE ON attempts
WHEN NEW.attempt_id != OLD.attempt_id
  OR NEW.operation_id != OLD.operation_id
  OR NEW.attempt_number != OLD.attempt_number
  OR NEW.owner_id != OLD.owner_id
  OR NEW.owner_generation != OLD.owner_generation
  OR NEW.revision != OLD.revision + 1
BEGIN SELECT RAISE(ABORT, 'invalid attempt revision update'); END;
CREATE TRIGGER attempts_no_delete BEFORE DELETE ON attempts
BEGIN SELECT RAISE(ABORT, 'attempt history is retained'); END;
CREATE TRIGGER operation_events_no_update BEFORE UPDATE ON operation_events
BEGIN SELECT RAISE(ABORT, 'operation events are append only'); END;
CREATE TRIGGER operation_events_no_delete BEFORE DELETE ON operation_events
BEGIN SELECT RAISE(ABORT, 'operation events are append only'); END;
CREATE TRIGGER operation_results_no_update BEFORE UPDATE ON operation_results
BEGIN SELECT RAISE(ABORT, 'operation results are immutable'); END;
CREATE TRIGGER operation_results_no_delete BEFORE DELETE ON operation_results
BEGIN SELECT RAISE(ABORT, 'operation results are immutable'); END;
CREATE TRIGGER operation_outbox_payload_guard BEFORE UPDATE ON operation_outbox
WHEN NEW.outbox_id != OLD.outbox_id
  OR NEW.root_namespace_id != OLD.root_namespace_id
  OR NEW.operation_id != OLD.operation_id
  OR NEW.attempt_id != OLD.attempt_id
  OR NEW.event_sequence != OLD.event_sequence
  OR NEW.consumer != OLD.consumer
  OR NEW.payload_json != OLD.payload_json
  OR NEW.created_at_ms != OLD.created_at_ms
  OR OLD.state != 'pending'
  OR NEW.state != 'acknowledged'
BEGIN SELECT RAISE(ABORT, 'invalid outbox acknowledgement'); END;
CREATE TRIGGER operation_outbox_no_delete BEFORE DELETE ON operation_outbox
BEGIN SELECT RAISE(ABORT, 'outbox history is retained'); END;
"#;

const SCHEMA_V2: &str = r#"
ALTER TABLE operations ADD COLUMN intent_digest TEXT
    CHECK (intent_digest IS NULL OR length(intent_digest) = 64);
DROP TRIGGER operations_immutable_update;
CREATE TABLE operation_owners (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id),
    owner_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE dispatch_claims (
    attempt_id TEXT PRIMARY KEY REFERENCES attempts(attempt_id),
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    owner_id TEXT NOT NULL,
    owner_generation INTEGER NOT NULL CHECK (owner_generation > 0),
    permit_nonce TEXT NOT NULL UNIQUE,
    claimed_at_ms INTEGER NOT NULL,
    effect_started INTEGER NOT NULL CHECK (effect_started IN (0, 1))
);
CREATE TABLE operation_call_mappings (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id),
    root_namespace_id TEXT NOT NULL REFERENCES journal_roots(root_namespace_id),
    session_id TEXT NOT NULL,
    caller_scope TEXT NOT NULL,
    caller_key TEXT NOT NULL,
    wire_tool_call_id TEXT,
    parent_operation_id TEXT,
    UNIQUE (root_namespace_id, caller_scope, caller_key),
    UNIQUE (root_namespace_id, session_id, wire_tool_call_id)
);
"#;

const SCHEMA_V2_GUARDS: &str = r#"
CREATE TRIGGER operations_intent_guard BEFORE INSERT ON operations
WHEN NEW.intent_digest IS NULL OR length(NEW.intent_digest) != 64
BEGIN SELECT RAISE(ABORT, 'operation intent digest is required'); END;
CREATE TRIGGER operations_immutable_update BEFORE UPDATE ON operations
BEGIN SELECT RAISE(ABORT, 'operation specifications are immutable'); END;
CREATE TRIGGER operation_owners_generation_guard BEFORE UPDATE ON operation_owners
WHEN NEW.operation_id != OLD.operation_id OR NEW.generation <= OLD.generation
BEGIN SELECT RAISE(ABORT, 'owner generation must increase'); END;
CREATE TRIGGER operation_owners_no_delete BEFORE DELETE ON operation_owners
BEGIN SELECT RAISE(ABORT, 'operation owner history is retained'); END;
CREATE TRIGGER dispatch_claims_effect_guard BEFORE UPDATE ON dispatch_claims
WHEN NEW.attempt_id != OLD.attempt_id
  OR NEW.operation_id != OLD.operation_id
  OR NEW.owner_id != OLD.owner_id
  OR NEW.owner_generation != OLD.owner_generation
  OR NEW.permit_nonce != OLD.permit_nonce
  OR NEW.claimed_at_ms != OLD.claimed_at_ms
  OR OLD.effect_started != 0
  OR NEW.effect_started != 1
BEGIN SELECT RAISE(ABORT, 'invalid dispatch effect latch'); END;
CREATE TRIGGER dispatch_claims_no_delete BEFORE DELETE ON dispatch_claims
BEGIN SELECT RAISE(ABORT, 'dispatch claims are retained'); END;
CREATE TRIGGER operation_call_mappings_no_update BEFORE UPDATE ON operation_call_mappings
BEGIN SELECT RAISE(ABORT, 'operation call mappings are immutable'); END;
CREATE TRIGGER operation_call_mappings_no_delete BEFORE DELETE ON operation_call_mappings
BEGIN SELECT RAISE(ABORT, 'operation call mappings are immutable'); END;
"#;

pub(super) fn initialize(
    connection: &mut Connection,
    identity: &JournalIdentity,
) -> Result<(), JournalError> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(JournalError::UnsupportedDatabase(application_id));
    }
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => {
            initialize_v1(connection, identity, application_id)?;
            migrate_v2(connection)?;
        }
        INITIAL_SCHEMA_VERSION => {
            validate_v1(connection, identity, application_id)?;
            migrate_v2(connection)?;
        }
        SCHEMA_VERSION => {
            validate_v1(connection, identity, application_id)?;
            validate_v2(connection)?;
        }
        other => return Err(JournalError::UnsupportedSchema(other)),
    }
    integrity_check(connection)
}

fn initialize_v1(
    connection: &mut Connection,
    identity: &JournalIdentity,
    application_id: i64,
) -> Result<(), JournalError> {
    let user_objects: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type IN ('table', 'index', 'trigger', 'view')
         AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    if user_objects != 0 || application_id != 0 {
        return Err(JournalError::UnexpectedSchema);
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
    transaction.execute_batch(SCHEMA_V1)?;
    transaction.execute(
        "INSERT INTO journal_metadata
         (singleton, schema_version, journal_id, workspace_id, workspace_binding_version, created_at_ms)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        params![
            INITIAL_SCHEMA_VERSION,
            identity.journal_id.to_string(),
            identity.workspace.id.to_string(),
            identity.workspace.binding_version,
            super::store_support::now_unix_millis()
        ],
    )?;
    transaction.execute(
        "INSERT INTO journal_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![
            INITIAL_SCHEMA_VERSION,
            super::store_support::now_unix_millis()
        ],
    )?;
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(None, "user_version", INITIAL_SCHEMA_VERSION)?;
    transaction.commit()?;
    validate_v1(connection, identity, APPLICATION_ID)
}

fn validate_v1(
    connection: &Connection,
    identity: &JournalIdentity,
    application_id: i64,
) -> Result<(), JournalError> {
    if application_id != APPLICATION_ID {
        return Err(JournalError::UnsupportedDatabase(application_id));
    }
    let stored: Option<(u32, String, String, u32)> = connection
        .query_row(
            "SELECT schema_version, journal_id, workspace_id, workspace_binding_version
             FROM journal_metadata WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((schema, journal_id, workspace_id, binding_version)) = stored else {
        return Err(JournalError::Integrity(
            "journal metadata is missing".into(),
        ));
    };
    if schema != INITIAL_SCHEMA_VERSION {
        return Err(JournalError::UnsupportedSchema(schema));
    }
    if journal_id != identity.journal_id.to_string() {
        return Err(JournalError::JournalMismatch {
            expected: identity.journal_id.to_string(),
            actual: journal_id,
        });
    }
    if workspace_id != identity.workspace.id.to_string()
        || binding_version != identity.workspace.binding_version
    {
        return Err(JournalError::WorkspaceMismatch {
            expected: format!(
                "{}:{}",
                identity.workspace.id, identity.workspace.binding_version
            ),
            actual: format!("{workspace_id}:{binding_version}"),
        });
    }
    let migration: Option<u32> = connection
        .query_row(
            "SELECT version FROM journal_migrations WHERE version = ?1",
            [INITIAL_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()?;
    if migration != Some(INITIAL_SCHEMA_VERSION) {
        return Err(JournalError::Integrity(
            "schema migration record is missing".into(),
        ));
    }
    Ok(())
}

fn migrate_v2(connection: &mut Connection) -> Result<(), JournalError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
    transaction.execute_batch(SCHEMA_V2)?;

    let operations = {
        let mut statement = transaction.prepare(
            "SELECT operation_id, root_namespace_id, spec_json FROM operations
             ORDER BY operation_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for (operation_id, root_namespace_id, spec_json) in operations {
        let spec: OperationSpec = super::store_support::decode_json(&spec_json)?;
        let intent_digest = spec
            .intent_digest()
            .map_err(|error| JournalError::Serialization(error.to_string()))?
            .to_string();
        let scope = serde_json::to_value(spec.caller_key().scope)
            .map_err(|error| JournalError::Serialization(error.to_string()))?
            .as_str()
            .ok_or_else(|| JournalError::Serialization("invalid idempotency scope".into()))?
            .to_owned();
        transaction.execute(
            "UPDATE operations SET intent_digest = ?1 WHERE operation_id = ?2",
            params![intent_digest, operation_id],
        )?;
        let attempt_owner: (String, i64) = transaction.query_row(
            "SELECT owner_id, owner_generation FROM attempts
             WHERE operation_id = ?1 ORDER BY attempt_number DESC LIMIT 1",
            [&operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        transaction.execute(
            "INSERT INTO operation_owners (operation_id, owner_id, generation, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                operation_id,
                attempt_owner.0,
                attempt_owner.1,
                super::store_support::now_unix_millis()
            ],
        )?;
        let parent_id = spec.context().parent_operation_id.map(|id| id.to_string());
        let wire_call_id = spec.context().wire_tool_call_id.as_deref();
        transaction.execute(
            "INSERT INTO operation_call_mappings
             (operation_id, root_namespace_id, session_id, caller_scope, caller_key,
              wire_tool_call_id, parent_operation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                operation_id,
                root_namespace_id,
                spec.context().session_id,
                scope,
                spec.caller_key().key,
                wire_call_id,
                parent_id
            ],
        )?;
    }

    transaction.execute_batch(SCHEMA_V2_GUARDS)?;
    transaction.execute(
        "INSERT INTO journal_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![SCHEMA_VERSION, super::store_support::now_unix_millis()],
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    validate_v2(connection)
}

fn validate_v2(connection: &Connection) -> Result<(), JournalError> {
    let migration: Option<u32> = connection
        .query_row(
            "SELECT version FROM journal_migrations WHERE version = ?1",
            [SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()?;
    if migration != Some(SCHEMA_VERSION) {
        return Err(JournalError::Integrity(
            "schema migration record is missing".into(),
        ));
    }
    let invalid_records: i64 = connection.query_row(
        "SELECT
           (SELECT count(*) FROM operations WHERE intent_digest IS NULL OR length(intent_digest) != 64)
         + (SELECT abs((SELECT count(*) FROM operations) - (SELECT count(*) FROM operation_owners)))
         + (SELECT abs((SELECT count(*) FROM operations) - (SELECT count(*) FROM operation_call_mappings)))",
        [],
        |row| row.get(0),
    )?;
    if invalid_records != 0 {
        return Err(JournalError::Integrity(
            "operation coordinator metadata is incomplete".into(),
        ));
    }
    Ok(())
}

fn integrity_check(connection: &Connection) -> Result<(), JournalError> {
    let status: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if status != "ok" {
        return Err(JournalError::Integrity(status));
    }
    let foreign_key_errors: i64 =
        connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_errors != 0 {
        return Err(JournalError::Integrity(format!(
            "{foreign_key_errors} foreign key violations"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::operations::{
        transition_attempt, CallerType, EffectClass, EffectProfile, ExecutionOwner,
        ExecutionOwnerId, IdempotencyScope, JournalId, OperationAttempt, OperationContext,
        OperationEvent, OperationKind, RootNamespaceId, ScopedIdempotencyKey, WorkspaceId,
        WorkspaceIdentity,
    };
    use crate::runtime::{AgentId, RunId, TaskId};
    use serde_json::json;

    #[test]
    fn v1_upgrade_backfills_intent_owner_and_call_mapping_for_existing_operations() {
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let root = RootNamespaceId::new();
        let spec = OperationSpec::new(
            OperationContext {
                journal_id: identity.journal_id,
                root_namespace_id: root,
                session_id: "migration-session".to_owned(),
                runtime_run_id: RunId::new(),
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: Some(TaskId::new()),
                graph: None,
                workspace: identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: Some("migration-call".to_owned()),
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, "migration-key").unwrap(),
            OperationKind::CustomExternalAction,
            EffectProfile {
                classification: EffectClass::ExternalMutation,
                supports_idempotency_key: true,
                supports_postcondition_probe: false,
                supports_compensation: false,
                requires_live_owner: true,
            },
            json!({"action": "migrate"}),
            vec![],
        )
        .unwrap();
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 7).unwrap();
        let created = OperationAttempt::new(spec.operation_id(), 1, owner).unwrap();
        let event = OperationEvent::Persist;
        let attempt = transition_attempt(&created, 0, event.clone()).unwrap();
        let spec_json = serde_json::to_string(&spec).unwrap();
        let attempt_json = serde_json::to_string(&attempt).unwrap();
        let event_json = serde_json::to_string(&event).unwrap();
        let scope = serde_json::to_value(spec.caller_key().scope)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA_V1).unwrap();
        connection
            .execute(
                "INSERT INTO journal_metadata
                 (singleton, schema_version, journal_id, workspace_id, workspace_binding_version, created_at_ms)
                 VALUES (1, 1, ?1, ?2, 1, 1)",
                params![identity.journal_id.to_string(), identity.workspace.id.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO journal_migrations (version, applied_at_ms) VALUES (1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO journal_roots
                 (root_namespace_id, journal_id, workspace_id, workspace_binding_version, created_at_ms)
                 VALUES (?1, ?2, ?3, 1, 1)",
                params![
                    root.to_string(),
                    identity.journal_id.to_string(),
                    identity.workspace.id.to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operations
                 (operation_id, root_namespace_id, caller_scope, caller_key, payload_digest, spec_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                params![
                    spec.operation_id().to_string(),
                    root.to_string(),
                    scope,
                    spec.caller_key().key,
                    spec.payload_digest().to_string(),
                    spec_json
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO attempts
                 (attempt_id, operation_id, attempt_number, revision, owner_id, owner_generation, state, attempt_json, updated_at_ms)
                 VALUES (?1, ?2, 1, 1, ?3, 7, 'persisted', ?4, 1)",
                params![
                    attempt.attempt_id().to_string(),
                    spec.operation_id().to_string(),
                    owner.id.to_string(),
                    attempt_json
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operation_events
                 (root_namespace_id, operation_id, attempt_id, prior_revision, revision, event_json, created_at_ms)
                 VALUES (?1, ?2, ?3, 0, 1, ?4, 1)",
                params![
                    root.to_string(),
                    spec.operation_id().to_string(),
                    attempt.attempt_id().to_string(),
                    event_json
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", INITIAL_SCHEMA_VERSION)
            .unwrap();

        initialize(&mut connection, &identity).unwrap();

        let stored_intent: String = connection
            .query_row(
                "SELECT intent_digest FROM operations WHERE operation_id = ?1",
                [spec.operation_id().to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let stored_owner: (String, i64) = connection
            .query_row(
                "SELECT owner_id, generation FROM operation_owners WHERE operation_id = ?1",
                [spec.operation_id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mapping_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM operation_call_mappings
                 WHERE operation_id = ?1 AND session_id = 'migration-session'
                   AND wire_tool_call_id = 'migration-call'",
                [spec.operation_id().to_string()],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stored_intent, spec.intent_digest().unwrap().to_string());
        assert_eq!(stored_owner, (owner.id.to_string(), 7));
        assert_eq!(mapping_count, 1);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            SCHEMA_VERSION
        );

        connection.pragma_update(None, "application_id", 0).unwrap();
        assert!(matches!(
            initialize(&mut connection, &identity),
            Err(JournalError::UnsupportedDatabase(0))
        ));
    }
}

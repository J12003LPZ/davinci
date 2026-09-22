use super::store_api::{JournalError, JournalIdentity};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub(super) const SCHEMA_VERSION: u32 = 1;
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
        0 => initialize_v1(connection, identity, application_id)?,
        SCHEMA_VERSION => validate_v1(connection, identity, application_id)?,
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
            SCHEMA_VERSION,
            identity.journal_id.to_string(),
            identity.workspace.id.to_string(),
            identity.workspace.binding_version,
            super::store_support::now_unix_millis()
        ],
    )?;
    transaction.execute(
        "INSERT INTO journal_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![SCHEMA_VERSION, super::store_support::now_unix_millis()],
    )?;
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
    if schema != SCHEMA_VERSION {
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
            [SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()?;
    if migration != Some(SCHEMA_VERSION) {
        return Err(JournalError::Integrity(
            "schema migration record is missing".into(),
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

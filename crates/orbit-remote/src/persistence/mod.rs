//! Remote feature persistence over Orbit's shared SQLite connection.
//!
//! `orbit-store` owns connection lifecycle and the generic feature-migration
//! ledger. This module owns the registry schema contract, every registry SQL
//! statement and row codec, and the transaction boundaries that make the hub
//! registry a coherent domain.

use std::path::Path;

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use orbit_store::Store;
use orbit_store::sqlite::migration::{FeatureMigration, FeatureSchemaStatus};
use rusqlite::{Connection, OptionalExtension};

mod knowledge;
mod registry;
mod snapshot;

pub use knowledge::{
    HubKnowledgeAllocatorState, HubKnowledgeAllocatorStatus, HubKnowledgeRequestIdentityV1,
    KnowledgeAuthorityCutoverState, KnowledgeAuthorityCutoverStatus, KnowledgeWorkspaceInventory,
    LegacyKnowledgeId,
};

const REMOTE_SCHEMA_FEATURE: &str = "orbit-remote";
const REMOTE_SCHEMA_MIGRATIONS: &[FeatureMigration] = &[
    FeatureMigration::new(
        1,
        "adopt_global_v8_registry_schema",
        adopt_global_v8_registry_schema,
    ),
    FeatureMigration::new(
        2,
        "dormant_hub_knowledge_sequences",
        apply_dormant_hub_knowledge_sequences,
    ),
    FeatureMigration::new(
        3,
        "knowledge_authority_cutover_state",
        apply_knowledge_authority_cutover_state,
    ),
];

/// Persistence facade for Orbit's remote coordination domain.
///
/// Opening this facade first opens the configured Store database, then adopts
/// and validates the registry tables shipped by Store-global migrations
/// v5/v6/v8 into the independent `orbit-remote` feature schema. Future remote
/// schema changes append to [`REMOTE_SCHEMA_MIGRATIONS`] instead of the Store
/// global migration registry.
#[derive(Clone)]
pub struct RemoteStore {
    store: Store,
}

impl RemoteStore {
    /// Open exactly `database_path` and bring the Remote feature schema current.
    pub fn open(database_path: &Path) -> Result<Self, OrbitError> {
        Self::from_store(Store::open(database_path)?)
    }

    /// Open an isolated in-memory database and bring the Remote schema current.
    pub fn open_in_memory() -> Result<Self, OrbitError> {
        Self::from_store(Store::open_in_memory()?)
    }

    /// Adopt an already-open Store connection into the Remote persistence
    /// facade. This is primarily useful when a process shares one Store handle
    /// across feature facades; adoption remains validated and fallible.
    pub fn from_store(store: Store) -> Result<Self, OrbitError> {
        store.apply_feature_migrations(REMOTE_SCHEMA_FEATURE, REMOTE_SCHEMA_MIGRATIONS)?;
        Ok(Self { store })
    }

    /// Return the validated Remote feature-schema status.
    pub fn schema_status(&self) -> Result<FeatureSchemaStatus, OrbitError> {
        self.store
            .feature_schema_status(REMOTE_SCHEMA_FEATURE, REMOTE_SCHEMA_MIGRATIONS)
    }

    fn read<T, F>(&self, operation: F) -> Result<T, OrbitError>
    where
        F: FnOnce(&Connection) -> Result<T, OrbitError>,
    {
        self.store.with_read_connection(operation)
    }

    #[cfg(test)]
    fn connection(&self) -> std::sync::Arc<std::sync::Mutex<Connection>> {
        self.store.connection()
    }
}

/// Feature v1 does not create or rewrite registry data. It validates the
/// schema already shipped by immutable Store-global migrations v5/v6/v8 and
/// lets the generic feature-migration transaction record the ownership handoff.
fn adopt_global_v8_registry_schema(conn: &Connection) -> Result<(), OrbitError> {
    validate_table(
        conn,
        "hosts",
        &[
            column("machine_id", "TEXT", false, None, 1),
            column("host_id", "TEXT", true, None, 0),
            column("labels_json", "TEXT", true, Some("'[]'"), 0),
            column("status", "TEXT", true, None, 0),
            column("registered_at", "TEXT", true, None, 0),
            column("updated_at", "TEXT", true, None, 0),
            column("retired_at", "TEXT", false, None, 0),
            column("last_seen_at", "TEXT", false, None, 0),
        ],
        &[],
        &[
            "host_idtextnotnullunique",
            "check(length(machine_id)>0)",
            "check(length(host_id)>0)",
            "check(statusin('active','retired'))",
            "check(json_valid(labels_json)andjson_type(labels_json)='array')",
            "status='active'andretired_atisnull",
            "status='retired'andretired_atisnotnull",
        ],
    )?;
    validate_table(
        conn,
        "host_aliases",
        &[
            column("host_id", "TEXT", false, None, 1),
            column("machine_id", "TEXT", true, None, 0),
            column("created_at", "TEXT", true, None, 0),
            column("warning", "TEXT", true, None, 0),
        ],
        &[foreign_key("machine_id", "hosts", "machine_id")],
        &["check(length(host_id)>0)", "check(length(warning)>0)"],
    )?;
    validate_table(
        conn,
        "workspace_ownership",
        &[
            column("workspace_id", "TEXT", false, None, 1),
            column("owner_machine_id", "TEXT", true, None, 0),
            column("bound_at", "TEXT", true, None, 0),
            column("updated_at", "TEXT", true, None, 0),
        ],
        &[foreign_key("owner_machine_id", "hosts", "machine_id")],
        &["check(length(workspace_id)>0)"],
    )?;
    validate_table(
        conn,
        "host_workspace_presence",
        &[
            column("machine_id", "TEXT", true, None, 1),
            column("workspace_id", "TEXT", true, None, 2),
            column("root", "TEXT", true, None, 0),
            column("last_verified", "TEXT", true, None, 0),
        ],
        &[foreign_key("machine_id", "hosts", "machine_id")],
        &[
            "primarykey(machine_id,workspace_id)",
            "check(length(workspace_id)>0)",
            "check(length(root)>0)",
        ],
    )?;
    validate_table(
        conn,
        "workspace_execution_profiles",
        &[
            column("workspace_id", "TEXT", false, None, 1),
            column("owner_machine_id", "TEXT", true, None, 0),
            column("generation", "INTEGER", true, None, 0),
            column("payload_json", "TEXT", true, None, 0),
            column("received_at", "TEXT", true, None, 0),
        ],
        &[
            foreign_key("owner_machine_id", "hosts", "machine_id"),
            foreign_key("workspace_id", "workspace_ownership", "workspace_id"),
        ],
        &[
            "generationintegernotnullcheck(generation>=1)",
            "check(json_valid(payload_json)andjson_type(payload_json)='object')",
        ],
    )?;
    validate_table(
        conn,
        "hub_registry_metadata",
        &[
            column("id", "INTEGER", false, None, 1),
            column("hub_machine_id", "TEXT", false, None, 0),
            column("registry_revision", "INTEGER", true, Some("0"), 0),
            column("updated_at", "TEXT", true, None, 0),
        ],
        &[],
        &[
            "integerprimarykeycheck(id=0)",
            "typeof(registry_revision)='integer'",
            "registry_revision>=0",
            "registry_revision<=9223372036854775807",
            "check(hub_machine_idisnullorlength(hub_machine_id)>0)",
        ],
    )?;

    for (index, definition) in [
        ("idx_hosts_status_host_id", "onhosts(status,host_id)"),
        (
            "idx_host_aliases_machine_id",
            "onhost_aliases(machine_id,created_at)",
        ),
        (
            "idx_workspace_ownership_owner",
            "onworkspace_ownership(owner_machine_id,workspace_id)",
        ),
        (
            "idx_host_workspace_presence_workspace",
            "onhost_workspace_presence(workspace_id,machine_id)",
        ),
    ] {
        require_schema_definition(conn, "index", index, &[definition])?;
    }

    for (trigger, fragments) in [
        (
            "hosts_host_id_not_alias_insert",
            &[
                "beforeinsertonhosts",
                "host_id=new.host_id",
                "reservedbyapermanentalias",
            ][..],
        ),
        (
            "hosts_host_id_not_alias_update",
            &[
                "beforeupdateofhost_idonhosts",
                "host_id=new.host_id",
                "reservedbyapermanentalias",
            ][..],
        ),
        (
            "host_alias_not_current_name_insert",
            &[
                "beforeinsertonhost_aliases",
                "host_id=new.host_id",
                "conflictswithacurrenthost_id",
            ][..],
        ),
        (
            "host_aliases_immutable_update",
            &["beforeupdateonhost_aliases", "hostaliasesareimmutable"][..],
        ),
        (
            "host_aliases_immutable_delete",
            &["beforedeleteonhost_aliases", "hostaliasesarepermanent"][..],
        ),
        (
            "execution_profile_owner_matches_insert",
            &[
                "beforeinsertonworkspace_execution_profiles",
                "o.workspace_id=new.workspace_id",
                "o.owner_machine_id=new.owner_machine_id",
                "executionprofileownerdoesnotmatchworkspaceownership",
            ][..],
        ),
        (
            "execution_profile_owner_matches_update",
            &[
                "beforeupdateofworkspace_id,owner_machine_idonworkspace_execution_profiles",
                "o.workspace_id=new.workspace_id",
                "o.owner_machine_id=new.owner_machine_id",
                "executionprofileownerdoesnotmatchworkspaceownership",
            ][..],
        ),
    ] {
        require_schema_definition(conn, "trigger", trigger, fragments)?;
    }

    let singleton_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM hub_registry_metadata WHERE id = 0",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            OrbitError::Migration(format!(
                "cannot adopt orbit-remote schema: read hub registry singleton: {error}"
            ))
        })?;
    if singleton_rows != 1 {
        return Err(OrbitError::Migration(format!(
            "cannot adopt orbit-remote schema: expected exactly one hub_registry_metadata row with id 0, found {singleton_rows}"
        )));
    }

    Ok(())
}

/// Remote v2 installs the hub-global ADR/learning allocation substrate in a
/// deliberately dormant state. Source discovery and activation are explicit
/// service operations: merely opening the Remote facade never scans a
/// checkout, advances a sequence, or changes an existing caller path.
fn apply_dormant_hub_knowledge_sequences(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
        CREATE TABLE hub_knowledge_allocator_state (
            id                    INTEGER PRIMARY KEY CHECK (id = 0),
            status                TEXT NOT NULL CHECK (status IN ('dormant', 'active')),
            activation_generation INTEGER NOT NULL DEFAULT 0
                                      CHECK (activation_generation >= 0),
            activated_at          TEXT,
            updated_at            TEXT NOT NULL,
            CHECK (
                (status = 'dormant' AND activated_at IS NULL)
                OR (status = 'active' AND activated_at IS NOT NULL)
            )
        );

        INSERT INTO hub_knowledge_allocator_state(
            id, status, activation_generation, activated_at, updated_at
        ) VALUES (0, 'dormant', 0, NULL, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

        CREATE TABLE hub_knowledge_sequences (
            kind          TEXT PRIMARY KEY CHECK (kind IN ('adr', 'learning')),
            next_sequence INTEGER NOT NULL
                          CHECK (next_sequence >= 1 AND next_sequence <= 4294967296),
            updated_at    TEXT NOT NULL
        );

        INSERT INTO hub_knowledge_sequences(kind, next_sequence, updated_at) VALUES
            ('adr', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            ('learning', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

        CREATE TABLE hub_knowledge_ids (
            kind          TEXT NOT NULL CHECK (kind IN ('adr', 'learning')),
            id            TEXT NOT NULL,
            workspace_id  TEXT NOT NULL CHECK (length(workspace_id) > 0),
            sequence      INTEGER NOT NULL CHECK (sequence >= 1 AND sequence <= 4294967295),
            origin        TEXT NOT NULL CHECK (origin IN ('legacy', 'allocated')),
            evidence_json TEXT NOT NULL
                          CHECK (json_valid(evidence_json) AND json_type(evidence_json) = 'array'),
            recorded_at   TEXT NOT NULL,
            PRIMARY KEY(kind, id),
            UNIQUE(kind, sequence),
            UNIQUE(workspace_id, kind, id)
        );

        CREATE INDEX hub_knowledge_ids_workspace
            ON hub_knowledge_ids(workspace_id, kind, sequence);

        CREATE TABLE hub_knowledge_workspace_reconciliation (
            workspace_id          TEXT PRIMARY KEY CHECK (length(workspace_id) > 0),
            source_digest         TEXT NOT NULL CHECK (length(source_digest) > 0),
            source_count          INTEGER NOT NULL CHECK (source_count >= 0),
            adr_max               INTEGER NOT NULL CHECK (adr_max >= 0 AND adr_max <= 4294967295),
            learning_max          INTEGER NOT NULL
                                      CHECK (learning_max >= 0 AND learning_max <= 4294967295),
            reconciliation_generation INTEGER NOT NULL
                                      CHECK (reconciliation_generation >= 1),
            reconciled_at         TEXT NOT NULL
        );

        CREATE TABLE hub_knowledge_allocation_ledger (
            mcp_call_id          TEXT PRIMARY KEY CHECK (length(mcp_call_id) > 0),
            workspace_id        TEXT NOT NULL CHECK (length(workspace_id) > 0),
            kind                TEXT NOT NULL CHECK (kind IN ('adr', 'learning')),
            id                  TEXT NOT NULL,
            sequence            INTEGER NOT NULL CHECK (sequence >= 1 AND sequence <= 4294967295),
            request_identity_json TEXT NOT NULL
                                CHECK (json_valid(request_identity_json)
                                       AND json_type(request_identity_json) = 'object'),
            allocated_at        TEXT NOT NULL,
            FOREIGN KEY(kind, id) REFERENCES hub_knowledge_ids(kind, id)
                ON UPDATE RESTRICT ON DELETE RESTRICT,
            UNIQUE(workspace_id, kind, id)
        );

        CREATE INDEX hub_knowledge_allocation_lookup
            ON hub_knowledge_allocation_ledger(workspace_id, kind, id);

        CREATE TRIGGER hub_knowledge_allocation_ledger_immutable_update
        BEFORE UPDATE ON hub_knowledge_allocation_ledger
        BEGIN
            SELECT RAISE(ABORT, 'hub knowledge allocation ledger is immutable');
        END;

        CREATE TRIGGER hub_knowledge_allocation_ledger_immutable_delete
        BEFORE DELETE ON hub_knowledge_allocation_ledger
        BEGIN
            SELECT RAISE(ABORT, 'hub knowledge allocation ledger is immutable');
        END;
        "#,
    )
    .map_err(|error| {
        OrbitError::Migration(format!(
            "apply orbit-remote dormant hub knowledge sequence schema: {error}"
        ))
    })?;
    Ok(())
}

/// Remote v3 records F3's exclusive, forward-only authority cutover.  This is
/// separate from allocator activation so a process restart can distinguish an
/// untouched installation from an interrupted reconciliation and resume it
/// without re-enabling compatibility authoring.
fn apply_knowledge_authority_cutover_state(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
        CREATE TABLE hub_knowledge_cutover_state (
            id          INTEGER PRIMARY KEY CHECK (id = 0),
            status      TEXT NOT NULL CHECK (
                status IN ('pre-activation', 'reconciling', 'active', 'failed-incomplete')
            ),
            generation  INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
            last_error  TEXT,
            updated_at  TEXT NOT NULL,
            CHECK (
                (status = 'failed-incomplete' AND last_error IS NOT NULL)
                OR (status != 'failed-incomplete' AND last_error IS NULL)
            )
        );

        INSERT INTO hub_knowledge_cutover_state(
            id, status, generation, last_error, updated_at
        )
        SELECT 0,
               CASE status WHEN 'active' THEN 'active' ELSE 'pre-activation' END,
               activation_generation,
               NULL,
               updated_at
        FROM hub_knowledge_allocator_state
        WHERE id = 0;
        "#,
    )
    .map_err(|error| {
        OrbitError::Migration(format!(
            "apply orbit-remote knowledge authority cutover schema: {error}"
        ))
    })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ColumnContract {
    name: String,
    data_type: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key_order: u32,
}

fn column(
    name: &str,
    data_type: &str,
    not_null: bool,
    default_value: Option<&str>,
    primary_key_order: u32,
) -> ColumnContract {
    ColumnContract {
        name: name.to_string(),
        data_type: data_type.to_string(),
        not_null,
        default_value: default_value.map(ToString::to_string),
        primary_key_order,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ForeignKeyContract {
    from_column: String,
    target_table: String,
    target_column: String,
    on_update: String,
    on_delete: String,
    match_name: String,
}

fn foreign_key(from_column: &str, target_table: &str, target_column: &str) -> ForeignKeyContract {
    ForeignKeyContract {
        from_column: from_column.to_string(),
        target_table: target_table.to_string(),
        target_column: target_column.to_string(),
        on_update: "RESTRICT".to_string(),
        on_delete: "RESTRICT".to_string(),
        match_name: "NONE".to_string(),
    }
}

fn validate_table(
    conn: &Connection,
    table: &str,
    expected_columns: &[ColumnContract],
    expected_foreign_keys: &[ForeignKeyContract],
    definition_fragments: &[&str],
) -> Result<(), OrbitError> {
    require_schema_definition(conn, "table", table, definition_fragments)?;

    let mut statement = conn
        .prepare(
            "SELECT name, type, \"notnull\", dflt_value, pk
             FROM pragma_table_info(?1) ORDER BY cid",
        )
        .map_err(|error| adoption_error(format!("inspect columns for table '{table}': {error}")))?;
    let rows = statement
        .query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| adoption_error(format!("read columns for table '{table}': {error}")))?;
    let actual_columns = rows
        .map(|row| {
            let (name, data_type, not_null, default_value, primary_key_order) = row
                .map_err(|error| adoption_error(format!("read table '{table}' column: {error}")))?;
            Ok(ColumnContract {
                name,
                data_type: data_type.to_ascii_uppercase(),
                not_null: not_null != 0,
                default_value,
                primary_key_order: u32::try_from(primary_key_order).map_err(|error| {
                    adoption_error(format!(
                        "table '{table}' has invalid primary-key order: {error}"
                    ))
                })?,
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;
    if actual_columns != expected_columns {
        return Err(adoption_error(format!(
            "shipped table '{table}' column contract differs: expected {expected_columns:?}, found {actual_columns:?}"
        )));
    }

    validate_foreign_keys(conn, table, expected_foreign_keys)
}

fn validate_foreign_keys(
    conn: &Connection,
    table: &str,
    expected: &[ForeignKeyContract],
) -> Result<(), OrbitError> {
    let mut statement = conn
        .prepare(
            "SELECT \"from\", \"table\", \"to\", on_update, on_delete, \"match\"
             FROM pragma_foreign_key_list(?1)",
        )
        .map_err(|error| {
            adoption_error(format!("inspect foreign keys for table '{table}': {error}"))
        })?;
    let rows = statement
        .query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| {
            adoption_error(format!("read foreign keys for table '{table}': {error}"))
        })?;
    let mut actual = rows
        .map(|row| {
            let (from_column, target_table, target_column, on_update, on_delete, match_name) = row
                .map_err(|error| {
                    adoption_error(format!("read table '{table}' foreign key: {error}"))
                })?;
            Ok(ForeignKeyContract {
                from_column,
                target_table,
                target_column,
                on_update: on_update.to_ascii_uppercase(),
                on_delete: on_delete.to_ascii_uppercase(),
                match_name: match_name.to_ascii_uppercase(),
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    if actual != expected {
        return Err(adoption_error(format!(
            "shipped table '{table}' foreign-key contract differs: expected {expected:?}, found {actual:?}"
        )));
    }
    Ok(())
}

fn require_schema_definition(
    conn: &Connection,
    object_type: &str,
    name: &str,
    required_fragments: &[&str],
) -> Result<(), OrbitError> {
    let definition: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
            [object_type, name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| adoption_error(format!("inspect {object_type} '{name}': {error}")))?;
    let definition = definition.ok_or_else(|| {
        adoption_error(format!(
            "required shipped {object_type} '{name}' is missing or has no SQL definition"
        ))
    })?;
    let canonical = canonical_sql(&definition);
    for fragment in required_fragments {
        let fragment = canonical_sql(fragment);
        if !canonical.contains(&fragment) {
            return Err(adoption_error(format!(
                "shipped {object_type} '{name}' definition is missing required contract fragment '{fragment}'"
            )));
        }
    }
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn adoption_error(detail: String) -> OrbitError {
    OrbitError::Migration(format!("cannot adopt orbit-remote schema: {detail}"))
}

fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(parsed.with_timezone(&Utc))
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests;

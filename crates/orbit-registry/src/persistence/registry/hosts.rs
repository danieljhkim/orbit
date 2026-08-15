//! Host registration, naming, lifecycle, and row codecs.

use std::collections::BTreeSet;
use std::str::FromStr;

use orbit_common::types::{
    HostAlias, HostNameResolution, HostRecord, HostRegistration, HostStatus, OrbitError,
    validate_host_id, validate_machine_id, validate_registry_identifier,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::super::RegistryStore;

const HOST_COLUMNS: &str = "machine_id, host_id, labels_json, status, registered_at, \
                            updated_at, retired_at, last_seen_at";
const ALIAS_COLUMNS: &str = "host_id, machine_id, created_at, warning";

impl RegistryStore {
    /// Register a stable machine declaration. Repeating an active declaration
    /// with the same name and labels returns the existing row without changing
    /// timestamps. A changed name, changed labels, or retired row is an
    /// incompatible declaration and cannot silently rename/reactivate it.
    pub fn register_host(&self, registration: &HostRegistration) -> Result<HostRecord, OrbitError> {
        validate_registration(registration)?;
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let (host, inserted) = register_host_in_transaction(tx.connection(), registration)?;
                if inserted {
                    advance_registry_revision(tx.connection())?;
                }
                Ok(host)
            })
    }

    /// Atomically register the singular hub host and stamp its immutable
    /// `machine_id` into the registry snapshot metadata. A fresh host row and
    /// the first hub stamp are one snapshot-visible mutation and therefore
    /// advance `registry_revision` exactly once. Repeating the same declaration
    /// is a no-op; a different configured hub fails before inserting a host.
    pub fn register_hub(&self, registration: &HostRegistration) -> Result<HostRecord, OrbitError> {
        validate_registration(registration)?;
        self.store.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let configured_hub: Option<String> = tx
                .connection()
                .query_row(
                    "SELECT hub_machine_id FROM hub_registry_metadata WHERE id = 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            if let Some(existing) = configured_hub.as_deref()
                && existing != registration.machine_id
            {
                return Err(OrbitError::InvalidInput(format!(
                    "hub identity is already configured as machine_id '{existing}'; a second hub is not supported in v1"
                )));
            }

            let (host, inserted) = register_host_in_transaction(tx.connection(), registration)?;
            let stamped = if configured_hub.is_none() {
                let updated = tx
                    .connection()
                    .execute(
                        "UPDATE hub_registry_metadata
                         SET hub_machine_id = ?1, updated_at = ?2 WHERE id = 0",
                        params![registration.machine_id, super::super::now_string()],
                    )
                    .map_err(|error| OrbitError::Store(format!("stamp hub machine_id: {error}")))?;
                if updated != 1 {
                    return Err(OrbitError::Store(
                        "hub registry metadata singleton row is missing".to_string(),
                    ));
                }
                true
            } else {
                false
            };

            if inserted || stamped {
                advance_registry_revision(tx.connection())?;
            }
            Ok(host)
        })
    }

    /// Rename an active machine and atomically preserve its old name as a
    /// permanent tombstone alias. Renaming to its current name is a no-op;
    /// every other current, retired, or alias name is unavailable.
    pub fn validate_host_rename(
        &self,
        machine_id: &str,
        new_host_id: &str,
    ) -> Result<HostRecord, OrbitError> {
        validate_machine_id(machine_id)?;
        validate_host_id(new_host_id)?;
        self.read(|conn| validate_host_rename_in_connection(conn, machine_id, new_host_id))
    }

    /// Rename an active machine and atomically preserve its old name as a
    /// permanent tombstone alias. Renaming to its current name is a no-op;
    /// every other current, retired, or alias name is unavailable. Callers
    /// coordinating a machine-local identity file may first call
    /// [`Self::validate_host_rename`]; this transaction deliberately repeats
    /// the same validation so a concurrent registry change still fails closed.
    pub fn rename_host(
        &self,
        machine_id: &str,
        new_host_id: &str,
    ) -> Result<HostRecord, OrbitError> {
        validate_machine_id(machine_id)?;
        validate_host_id(new_host_id)?;
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let existing =
                    validate_host_rename_in_connection(tx.connection(), machine_id, new_host_id)?;
                if existing.host_id == new_host_id {
                    return Ok(existing);
                }

                let old_host_id = existing.host_id;
                let now = super::super::now_string();
                tx.connection()
                    .execute(
                        "UPDATE hosts SET host_id = ?2, updated_at = ?3 WHERE machine_id = ?1",
                        params![machine_id, new_host_id, now],
                    )
                    .map_err(|error| host_mutation_error("rename", new_host_id, error))?;
                let warning = format!(
                    "host_id '{old_host_id}' is a permanent tombstone alias for machine_id \
                 '{machine_id}' and cannot be reused"
                );
                tx.connection()
                    .execute(
                        "INSERT INTO host_aliases(host_id, machine_id, created_at, warning)
                     VALUES (?1, ?2, ?3, ?4)",
                        params![old_host_id, machine_id, now, warning],
                    )
                    .map_err(|error| host_mutation_error("preserve alias", &old_host_id, error))?;
                advance_registry_revision(tx.connection())?;

                host_by_machine_id(tx.connection(), machine_id)?.ok_or_else(|| {
                    OrbitError::Store(format!(
                        "renamed host_id '{old_host_id}' but could not read machine_id \
                     '{machine_id}' back"
                    ))
                })
            })
    }

    /// Retire a machine without deleting its identity or aliases. Repeating a
    /// retirement returns the existing row without moving lifecycle times. The
    /// configured v1 hub guard is read and enforced inside the same immediate
    /// transaction as the lifecycle mutation.
    pub fn retire_host(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        validate_machine_id(machine_id)?;
        self.store.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let existing = host_by_machine_id(tx.connection(), machine_id)?.ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "cannot retire unknown host machine_id '{machine_id}'"
                ))
            })?;
            if existing.status == HostStatus::Retired {
                return Ok(existing);
            }
            let configured_hub: Option<String> = tx
                .connection()
                .query_row(
                    "SELECT hub_machine_id FROM hub_registry_metadata WHERE id = 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            if configured_hub.as_deref() == Some(machine_id) {
                return Err(OrbitError::InvalidInput(format!(
                    "machine_id '{machine_id}' is the currently configured hub and cannot retire itself in v1"
                )));
            }
            let now = super::super::now_string();
            tx.connection()
                .execute(
                    "UPDATE hosts
                     SET status = ?2, retired_at = ?3, updated_at = ?3
                     WHERE machine_id = ?1",
                    params![machine_id, HostStatus::Retired.as_str(), now],
                )
                .map_err(|error| host_mutation_error("retire", &existing.host_id, error))?;
            advance_registry_revision(tx.connection())?;
            host_by_machine_id(tx.connection(), machine_id)?.ok_or_else(|| {
                OrbitError::Store(format!(
                    "retired host_id '{}' but could not read it back",
                    existing.host_id
                ))
            })
        })
    }

    /// Look up one machine by its immutable ID.
    pub fn get_host(&self, machine_id: &str) -> Result<Option<HostRecord>, OrbitError> {
        validate_machine_id(machine_id)?;
        self.read(|conn| host_by_machine_id(conn, machine_id))
    }

    /// Enumerate active machines in stable human-name order. Retired rows stay
    /// durable but are deliberately absent from this projection.
    pub fn list_active_hosts(&self) -> Result<Vec<HostRecord>, OrbitError> {
        self.read(|conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {HOST_COLUMNS} FROM hosts WHERE status = ?1 ORDER BY host_id ASC"
                ))
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            let rows = statement
                .query_map([HostStatus::Active.as_str()], host_row)
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            collect_hosts(rows)
        })
    }

    /// List the permanent name history of a machine, oldest first.
    pub fn list_host_aliases(&self, machine_id: &str) -> Result<Vec<HostAlias>, OrbitError> {
        validate_machine_id(machine_id)?;
        self.read(|conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {ALIAS_COLUMNS} FROM host_aliases
                     WHERE machine_id = ?1 ORDER BY created_at ASC, host_id ASC"
                ))
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            let rows = statement
                .query_map([machine_id], alias_row)
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            collect_aliases(rows)
        })
    }

    /// Resolve a current or historical human name without guessing. Retired
    /// machines remain resolvable, aliases retain warning metadata, unknown is
    /// explicit, and inconsistent cross-table matches fail closed as a typed
    /// collision outcome.
    pub fn resolve_host_id(&self, host_id: &str) -> Result<HostNameResolution, OrbitError> {
        validate_host_id(host_id)?;
        self.read(|conn| resolve_host_id_in_connection(conn, host_id))
    }
}

fn resolve_host_id_in_connection(
    conn: &Connection,
    host_id: &str,
) -> Result<HostNameResolution, OrbitError> {
    let mut statement = conn
        .prepare(
            "SELECT
                h.machine_id, h.host_id, h.labels_json, h.status,
                h.registered_at, h.updated_at, h.retired_at, h.last_seen_at,
                NULL, NULL, NULL, NULL
             FROM hosts h WHERE h.host_id = ?1
             UNION ALL
             SELECT
                h.machine_id, h.host_id, h.labels_json, h.status,
                h.registered_at, h.updated_at, h.retired_at, h.last_seen_at,
                a.host_id, a.machine_id, a.created_at, a.warning
             FROM host_aliases a
             JOIN hosts h ON h.machine_id = a.machine_id
             WHERE a.host_id = ?1",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([host_id], resolution_row)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut matches = Vec::new();
    for row in rows {
        let raw = row.map_err(|error| OrbitError::Store(error.to_string()))?;
        matches.push((
            HostRecord::try_from(raw.host)?,
            raw.alias.map(HostAlias::try_from).transpose()?,
        ));
    }

    if matches.is_empty() {
        return Ok(HostNameResolution::Unknown {
            host_id: host_id.to_string(),
        });
    }
    if matches.len() > 1 {
        let mut machine_ids: Vec<String> = matches
            .into_iter()
            .map(|(host, _)| host.machine_id)
            .collect();
        machine_ids.sort();
        machine_ids.dedup();
        return Ok(HostNameResolution::Collision {
            host_id: host_id.to_string(),
            machine_ids,
        });
    }

    let (host, alias) = matches.pop().ok_or_else(|| {
        OrbitError::Store(format!(
            "host_id '{host_id}' resolution lost its only match"
        ))
    })?;
    match (host.status, alias) {
        (HostStatus::Active, None) => Ok(HostNameResolution::Active { host }),
        (HostStatus::Active, Some(alias)) => Ok(HostNameResolution::Alias { host, alias }),
        (HostStatus::Retired, alias) => Ok(HostNameResolution::Retired { host, alias }),
    }
}

fn register_host_in_transaction(
    conn: &Connection,
    registration: &HostRegistration,
) -> Result<(HostRecord, bool), OrbitError> {
    if let Some(existing) = host_by_machine_id(conn, &registration.machine_id)? {
        ensure_compatible_registration(&existing, registration)?;
        return Ok((existing, false));
    }

    ensure_name_available(conn, &registration.host_id)?;
    let now = super::super::now_string();
    let labels_json = serde_json::to_string(&registration.labels)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    conn.execute(
        "INSERT INTO hosts(
            machine_id, host_id, labels_json, status, registered_at,
            updated_at, retired_at, last_seen_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, NULL, ?5)",
        params![
            registration.machine_id,
            registration.host_id,
            labels_json,
            HostStatus::Active.as_str(),
            now,
        ],
    )
    .map_err(|error| host_mutation_error("register", &registration.host_id, error))?;

    let host = host_by_machine_id(conn, &registration.machine_id)?.ok_or_else(|| {
        OrbitError::Store(format!(
            "registered host_id '{}' but could not read it back",
            registration.host_id
        ))
    })?;
    Ok((host, true))
}

/// Advance the hub-global registry revision by exactly one inside the caller's
/// write transaction. Callers invoke this only on the snapshot-visible mutation
/// branch, never on an idempotent no-op return.
pub(super) fn advance_registry_revision(conn: &Connection) -> Result<(), OrbitError> {
    let updated = conn
        .execute(
            "UPDATE hub_registry_metadata
             SET registry_revision = registry_revision + 1, updated_at = ?1
             WHERE id = 0
               AND typeof(registry_revision) = 'integer'
               AND registry_revision < 9223372036854775807",
            params![super::super::now_string()],
        )
        .map_err(|error| OrbitError::Store(format!("advance registry revision: {error}")))?;
    if updated != 1 {
        let state: Option<(String, i64)> = conn
            .query_row(
                "SELECT typeof(registry_revision), registry_revision
                 FROM hub_registry_metadata WHERE id = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        if let Some((storage_class, revision)) = state {
            return Err(OrbitError::Store(format!(
                "cannot advance registry revision beyond SQLite INTEGER range (storage class {storage_class}, current value {revision})"
            )));
        }
        return Err(OrbitError::Store(
            "hub registry metadata singleton row is missing".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn require_active_host(
    conn: &Connection,
    machine_id: &str,
) -> Result<HostRecord, OrbitError> {
    let host = host_by_machine_id(conn, machine_id)?.ok_or_else(|| {
        OrbitError::InvalidInput(format!("unknown host machine_id '{machine_id}'"))
    })?;
    if host.status != HostStatus::Active {
        return Err(OrbitError::InvalidInput(format!(
            "host machine_id '{machine_id}' is retired"
        )));
    }
    Ok(host)
}

fn validate_registration(registration: &HostRegistration) -> Result<(), OrbitError> {
    validate_machine_id(&registration.machine_id)?;
    validate_host_id(&registration.host_id)?;
    for label in &registration.labels {
        validate_registry_identifier("host label", label)?;
    }
    Ok(())
}

fn ensure_compatible_registration(
    existing: &HostRecord,
    registration: &HostRegistration,
) -> Result<(), OrbitError> {
    if existing.status == HostStatus::Retired {
        return Err(OrbitError::InvalidInput(format!(
            "host_id '{}' for machine_id '{}' is retired; re-registration cannot reactivate it",
            existing.host_id, existing.machine_id
        )));
    }
    if existing.host_id != registration.host_id {
        return Err(OrbitError::InvalidInput(format!(
            "machine_id '{}' is already registered as host_id '{}'; re-registration cannot \
             rename it to '{}' (use explicit rename)",
            existing.machine_id, existing.host_id, registration.host_id
        )));
    }
    if existing.labels != registration.labels {
        return Err(OrbitError::InvalidInput(format!(
            "host_id '{}' for machine_id '{}' is already registered with a different label \
             declaration",
            existing.host_id, existing.machine_id
        )));
    }
    Ok(())
}

fn validate_host_rename_in_connection(
    conn: &Connection,
    machine_id: &str,
    new_host_id: &str,
) -> Result<HostRecord, OrbitError> {
    let existing = host_by_machine_id(conn, machine_id)?.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "cannot rename unknown host machine_id '{machine_id}'"
        ))
    })?;
    if existing.status == HostStatus::Retired {
        return Err(OrbitError::InvalidInput(format!(
            "host_id '{}' is retired and cannot be renamed",
            existing.host_id
        )));
    }
    if existing.host_id != new_host_id {
        ensure_name_available(conn, new_host_id)?;
    }
    Ok(existing)
}

fn ensure_name_available(conn: &Connection, host_id: &str) -> Result<(), OrbitError> {
    if let Some(host) = host_by_host_id(conn, host_id)? {
        let lifecycle = if host.status == HostStatus::Retired {
            "retired"
        } else {
            "active"
        };
        return Err(OrbitError::InvalidInput(format!(
            "host_id '{host_id}' is already reserved by {lifecycle} machine_id '{}' and cannot \
             be reused",
            host.machine_id
        )));
    }
    if let Some(alias) = alias_by_host_id(conn, host_id)? {
        return Err(OrbitError::InvalidInput(format!(
            "host_id '{host_id}' is a permanent tombstone alias for machine_id '{}' and cannot \
             be reclaimed",
            alias.machine_id
        )));
    }
    Ok(())
}

fn host_by_machine_id(
    conn: &Connection,
    machine_id: &str,
) -> Result<Option<HostRecord>, OrbitError> {
    conn.query_row(
        &format!("SELECT {HOST_COLUMNS} FROM hosts WHERE machine_id = ?1"),
        [machine_id],
        host_row,
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(HostRecord::try_from)
    .transpose()
}

fn host_by_host_id(conn: &Connection, host_id: &str) -> Result<Option<HostRecord>, OrbitError> {
    conn.query_row(
        &format!("SELECT {HOST_COLUMNS} FROM hosts WHERE host_id = ?1"),
        [host_id],
        host_row,
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(HostRecord::try_from)
    .transpose()
}

fn alias_by_host_id(conn: &Connection, host_id: &str) -> Result<Option<HostAlias>, OrbitError> {
    conn.query_row(
        &format!("SELECT {ALIAS_COLUMNS} FROM host_aliases WHERE host_id = ?1"),
        [host_id],
        alias_row,
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(HostAlias::try_from)
    .transpose()
}

struct RawHostRow {
    machine_id: String,
    host_id: String,
    labels_json: String,
    status: String,
    registered_at: String,
    updated_at: String,
    retired_at: Option<String>,
    last_seen_at: Option<String>,
}

fn host_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawHostRow> {
    Ok(RawHostRow {
        machine_id: row.get(0)?,
        host_id: row.get(1)?,
        labels_json: row.get(2)?,
        status: row.get(3)?,
        registered_at: row.get(4)?,
        updated_at: row.get(5)?,
        retired_at: row.get(6)?,
        last_seen_at: row.get(7)?,
    })
}

impl TryFrom<RawHostRow> for HostRecord {
    type Error = OrbitError;

    fn try_from(row: RawHostRow) -> Result<Self, Self::Error> {
        let labels: BTreeSet<String> = serde_json::from_str(&row.labels_json).map_err(|error| {
            OrbitError::Store(format!(
                "host_id '{}' has invalid labels_json: {error}",
                row.host_id
            ))
        })?;
        let status = HostStatus::from_str(&row.status)
            .map_err(|error| OrbitError::Store(format!("host_id '{}': {error}", row.host_id)))?;
        Ok(Self {
            machine_id: row.machine_id,
            host_id: row.host_id,
            labels,
            status,
            registered_at: super::super::parse_timestamp(&row.registered_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            updated_at: super::super::parse_timestamp(&row.updated_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            retired_at: parse_optional_timestamp(row.retired_at)?,
            last_seen_at: parse_optional_timestamp(row.last_seen_at)?,
        })
    }
}

struct RawAliasRow {
    host_id: String,
    machine_id: String,
    created_at: String,
    warning: String,
}

struct RawResolutionRow {
    host: RawHostRow,
    alias: Option<RawAliasRow>,
}

fn resolution_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawResolutionRow> {
    let alias_host_id: Option<String> = row.get(8)?;
    let alias = match alias_host_id {
        Some(host_id) => Some(RawAliasRow {
            host_id,
            machine_id: row.get(9)?,
            created_at: row.get(10)?,
            warning: row.get(11)?,
        }),
        None => None,
    };
    Ok(RawResolutionRow {
        host: RawHostRow {
            machine_id: row.get(0)?,
            host_id: row.get(1)?,
            labels_json: row.get(2)?,
            status: row.get(3)?,
            registered_at: row.get(4)?,
            updated_at: row.get(5)?,
            retired_at: row.get(6)?,
            last_seen_at: row.get(7)?,
        },
        alias,
    })
}

fn alias_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAliasRow> {
    Ok(RawAliasRow {
        host_id: row.get(0)?,
        machine_id: row.get(1)?,
        created_at: row.get(2)?,
        warning: row.get(3)?,
    })
}

impl TryFrom<RawAliasRow> for HostAlias {
    type Error = OrbitError;

    fn try_from(row: RawAliasRow) -> Result<Self, Self::Error> {
        Ok(Self {
            alias_host_id: row.host_id,
            machine_id: row.machine_id,
            created_at: super::super::parse_timestamp(&row.created_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            warning: row.warning,
        })
    }
}

fn parse_optional_timestamp(
    value: Option<String>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, OrbitError> {
    value
        .map(|value| {
            super::super::parse_timestamp(&value)
                .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .transpose()
}

fn collect_hosts(
    rows: impl Iterator<Item = rusqlite::Result<RawHostRow>>,
) -> Result<Vec<HostRecord>, OrbitError> {
    rows.map(|row| {
        row.map_err(|error| OrbitError::Store(error.to_string()))?
            .try_into()
    })
    .collect()
}

fn collect_aliases(
    rows: impl Iterator<Item = rusqlite::Result<RawAliasRow>>,
) -> Result<Vec<HostAlias>, OrbitError> {
    rows.map(|row| {
        row.map_err(|error| OrbitError::Store(error.to_string()))?
            .try_into()
    })
    .collect()
}

fn host_mutation_error(operation: &str, host_id: &str, error: rusqlite::Error) -> OrbitError {
    OrbitError::Store(format!(
        "failed to {operation} host_id '{host_id}': {error}"
    ))
}

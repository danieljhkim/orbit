//! Durable host-registry core for the coordination hub [ORB-10255].
//!
//! The registry keys machines by immutable `machine_id`, keeps every current
//! and historical human `host_id` reserved forever, and records retirement
//! without deleting identity. Registration, rename, and retirement use
//! `BEGIN IMMEDIATE` transactions so collision preflight and mutation share a
//! single writer boundary.

use std::collections::BTreeSet;
use std::str::FromStr;

use orbit_common::types::{
    HostAlias, HostNameResolution, HostRecord, HostRegistration, HostStatus, OrbitError,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::Store;

const HOST_COLUMNS: &str = "machine_id, host_id, labels_json, status, registered_at, \
                            updated_at, retired_at, last_seen_at";
const ALIAS_COLUMNS: &str = "host_id, machine_id, created_at, warning";

impl Store {
    /// Register a stable machine declaration. Repeating an active declaration
    /// with the same name and labels returns the existing row without changing
    /// timestamps. A changed name, changed labels, or retired row is an
    /// incompatible declaration and cannot silently rename/reactivate it.
    pub fn register_host(&self, registration: &HostRegistration) -> Result<HostRecord, OrbitError> {
        validate_registration(registration)?;
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            if let Some(existing) = host_by_machine_id(&tx.tx, &registration.machine_id)? {
                ensure_compatible_registration(&existing, registration)?;
                return Ok(existing);
            }

            ensure_name_available(&tx.tx, &registration.host_id)?;
            let now = crate::now_string();
            let labels_json = serde_json::to_string(&registration.labels)
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            tx.tx
                .execute(
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

            host_by_machine_id(&tx.tx, &registration.machine_id)?.ok_or_else(|| {
                OrbitError::Store(format!(
                    "registered host_id '{}' but could not read it back",
                    registration.host_id
                ))
            })
        })
    }

    /// Rename an active machine and atomically preserve its old name as a
    /// permanent tombstone alias. Renaming to its current name is a no-op;
    /// every other current, retired, or alias name is unavailable.
    pub fn rename_host(
        &self,
        machine_id: &str,
        new_host_id: &str,
    ) -> Result<HostRecord, OrbitError> {
        validate_machine_id(machine_id)?;
        validate_host_id(new_host_id)?;
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let existing = host_by_machine_id(&tx.tx, machine_id)?.ok_or_else(|| {
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
            if existing.host_id == new_host_id {
                return Ok(existing);
            }
            ensure_name_available(&tx.tx, new_host_id)?;

            let old_host_id = existing.host_id;
            let now = crate::now_string();
            tx.tx
                .execute(
                    "UPDATE hosts SET host_id = ?2, updated_at = ?3 WHERE machine_id = ?1",
                    params![machine_id, new_host_id, now],
                )
                .map_err(|error| host_mutation_error("rename", new_host_id, error))?;
            let warning = format!(
                "host_id '{old_host_id}' is a permanent tombstone alias for machine_id \
                 '{machine_id}' and cannot be reused"
            );
            tx.tx
                .execute(
                    "INSERT INTO host_aliases(host_id, machine_id, created_at, warning)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![old_host_id, machine_id, now, warning],
                )
                .map_err(|error| host_mutation_error("preserve alias", &old_host_id, error))?;

            host_by_machine_id(&tx.tx, machine_id)?.ok_or_else(|| {
                OrbitError::Store(format!(
                    "renamed host_id '{old_host_id}' but could not read machine_id \
                     '{machine_id}' back"
                ))
            })
        })
    }

    /// Retire a machine without deleting its identity or aliases. Repeating a
    /// retirement returns the existing row without moving lifecycle times.
    pub fn retire_host(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        validate_machine_id(machine_id)?;
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let existing = host_by_machine_id(&tx.tx, machine_id)?.ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "cannot retire unknown host machine_id '{machine_id}'"
                ))
            })?;
            if existing.status == HostStatus::Retired {
                return Ok(existing);
            }
            let now = crate::now_string();
            tx.tx
                .execute(
                    "UPDATE hosts
                     SET status = ?2, retired_at = ?3, updated_at = ?3
                     WHERE machine_id = ?1",
                    params![machine_id, HostStatus::Retired.as_str(), now],
                )
                .map_err(|error| host_mutation_error("retire", &existing.host_id, error))?;
            host_by_machine_id(&tx.tx, machine_id)?.ok_or_else(|| {
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
        let conn = self.read()?;
        host_by_machine_id(&conn, machine_id)
    }

    /// Enumerate active machines in stable human-name order. Retired rows stay
    /// durable but are deliberately absent from this projection.
    pub fn list_active_hosts(&self) -> Result<Vec<HostRecord>, OrbitError> {
        let conn = self.read()?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT {HOST_COLUMNS} FROM hosts WHERE status = ?1 ORDER BY host_id ASC"
            ))
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = statement
            .query_map([HostStatus::Active.as_str()], host_row)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        collect_hosts(rows)
    }

    /// List the permanent name history of a machine, oldest first.
    pub fn list_host_aliases(&self, machine_id: &str) -> Result<Vec<HostAlias>, OrbitError> {
        validate_machine_id(machine_id)?;
        let conn = self.read()?;
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
    }

    /// Resolve a current or historical human name without guessing. Retired
    /// machines remain resolvable, aliases retain warning metadata, unknown is
    /// explicit, and inconsistent cross-table matches fail closed as a typed
    /// collision outcome.
    pub fn resolve_host_id(&self, host_id: &str) -> Result<HostNameResolution, OrbitError> {
        validate_host_id(host_id)?;
        let conn = self.read()?;
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
}

fn validate_registration(registration: &HostRegistration) -> Result<(), OrbitError> {
    validate_machine_id(&registration.machine_id)?;
    validate_host_id(&registration.host_id)?;
    for label in &registration.labels {
        validate_registry_text("host label", label)?;
    }
    Ok(())
}

fn validate_machine_id(machine_id: &str) -> Result<(), OrbitError> {
    validate_registry_text("machine_id", machine_id)
}

fn validate_host_id(host_id: &str) -> Result<(), OrbitError> {
    validate_registry_text("host_id", host_id)
}

fn validate_registry_text(field: &str, value: &str) -> Result<(), OrbitError> {
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    if value.trim() != value {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not contain leading or trailing whitespace"
        )));
    }
    if value.len() > 128 {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not exceed 128 bytes"
        )));
    }
    if value.chars().any(char::is_control) || value.contains(['/', '\\']) {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must be a logical registry identifier, not a path"
        )));
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
            registered_at: crate::parse_timestamp(&row.registered_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            updated_at: crate::parse_timestamp(&row.updated_at)
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
            created_at: crate::parse_timestamp(&row.created_at)
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
            crate::parse_timestamp(&value).map_err(|error| OrbitError::Store(error.to_string()))
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

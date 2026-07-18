//! Durable host-registry core for the coordination hub [ORB-10255].
//!
//! The registry keys machines by immutable `machine_id`, keeps every current
//! and historical human `host_id` reserved forever, and records retirement
//! without deleting identity. Registration, rename, and retirement use
//! `BEGIN IMMEDIATE` transactions so collision preflight and mutation share a
//! single writer boundary.

use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    ExecutionProfileV1, HostAlias, HostNameResolution, HostRecord, HostRegistration, HostStatus,
    HostWorkspacePresence, OrbitError, ProjectionFreshness, SanitizedExecutionProfile,
    SanitizedWorkspacePresence, StoredExecutionProfile, WorkspaceOwnership,
    WorkspacePresenceDeclaration,
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
            advance_registry_revision(&tx.tx)?;

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
            advance_registry_revision(&tx.tx)?;

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
            advance_registry_revision(&tx.tx)?;
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

    /// Declare the one durable owner of an existing logical workspace. The
    /// service layer validates logical-workspace existence; this store layer
    /// validates stable identifiers and active host lifecycle. Repeating the
    /// same binding is idempotent, while rebinding fails closed.
    pub fn bind_workspace_owner(
        &self,
        workspace_id: &str,
        owner_machine_id: &str,
    ) -> Result<WorkspaceOwnership, OrbitError> {
        validate_registry_text("workspace_id", workspace_id)?;
        validate_machine_id(owner_machine_id)?;
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            require_active_host(&tx.tx, owner_machine_id)?;
            if let Some(existing) = ownership_by_workspace(&tx.tx, workspace_id)? {
                if existing.owner_machine_id != owner_machine_id {
                    return Err(OrbitError::InvalidInput(format!(
                        "workspace_id '{workspace_id}' is already owned by machine_id '{}'; ownership migration must be explicit",
                        existing.owner_machine_id
                    )));
                }
                return Ok(existing);
            }
            let now = crate::now_string();
            tx.tx
                .execute(
                    "INSERT INTO workspace_ownership(
                        workspace_id, owner_machine_id, bound_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?3)",
                    params![workspace_id, owner_machine_id, now],
                )
                .map_err(|error| {
                    OrbitError::Store(format!(
                        "bind owner for workspace_id '{workspace_id}': {error}"
                    ))
                })?;
            advance_registry_revision(&tx.tx)?;
            ownership_by_workspace(&tx.tx, workspace_id)?.ok_or_else(|| {
                OrbitError::Store(format!(
                    "bound owner for workspace_id '{workspace_id}' but could not read it back"
                ))
            })
        })
    }

    pub fn get_workspace_ownership(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceOwnership>, OrbitError> {
        validate_registry_text("workspace_id", workspace_id)?;
        let conn = self.read()?;
        ownership_by_workspace(&conn, workspace_id)
    }

    /// Atomically replace one authenticated machine's full declared presence
    /// map and stamp that host's explicit `last_seen_at`. Roots stay confined
    /// to this private store projection.
    pub fn replace_host_workspace_presence(
        &self,
        caller_machine_id: &str,
        declarations: &[WorkspacePresenceDeclaration],
        received_at: DateTime<Utc>,
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        validate_machine_id(caller_machine_id)?;
        let mut workspace_ids = BTreeSet::new();
        for declaration in declarations {
            validate_registry_text("workspace_id", &declaration.workspace_id)?;
            if !workspace_ids.insert(declaration.workspace_id.as_str()) {
                return Err(OrbitError::InvalidInput(format!(
                    "presence declaration repeats workspace_id '{}'",
                    declaration.workspace_id
                )));
            }
            validate_presence_root(&declaration.root)?;
        }

        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            require_active_host(&tx.tx, caller_machine_id)?;
            tx.tx
                .execute(
                    "DELETE FROM host_workspace_presence WHERE machine_id = ?1",
                    [caller_machine_id],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            for declaration in declarations {
                let root = declaration.root.to_str().ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "presence root for workspace_id '{}' must be valid UTF-8",
                        declaration.workspace_id
                    ))
                })?;
                tx.tx
                    .execute(
                        "INSERT INTO host_workspace_presence(
                            machine_id, workspace_id, root, last_verified
                         ) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            caller_machine_id,
                            declaration.workspace_id,
                            root,
                            declaration.last_verified.to_rfc3339(),
                        ],
                    )
                    .map_err(|error| {
                        OrbitError::Store(format!(
                            "publish presence for workspace_id '{}': {error}",
                            declaration.workspace_id
                        ))
                    })?;
            }
            tx.tx
                .execute(
                    "UPDATE hosts SET last_seen_at = ?2 WHERE machine_id = ?1",
                    params![caller_machine_id, received_at.to_rfc3339()],
                )
                .map_err(|error| OrbitError::Store(format!("update host last_seen: {error}")))?;
            // Presence replacement always restamps the host receipt
            // (`last_seen_at` / `last_verified`), a freshness-visible change,
            // so the snapshot-visible revision advances every publish.
            advance_registry_revision(&tx.tx)?;
            presence_for_machine(&tx.tx, caller_machine_id)
        })
    }

    pub fn list_host_workspace_presence_private(
        &self,
        machine_id: &str,
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        validate_machine_id(machine_id)?;
        let conn = self.read()?;
        presence_for_machine(&conn, machine_id)
    }

    pub fn sanitized_workspace_presence(
        &self,
        machine_id: &str,
        workspace_id: &str,
        now: DateTime<Utc>,
        freshness_ttl: Duration,
    ) -> Result<SanitizedWorkspacePresence, OrbitError> {
        validate_machine_id(machine_id)?;
        validate_registry_text("workspace_id", workspace_id)?;
        validate_nonnegative_duration("presence freshness TTL", freshness_ttl)?;
        let conn = self.read()?;
        let owner_machine_id = ownership_by_workspace(&conn, workspace_id)?
            .map(|ownership| ownership.owner_machine_id);
        let presence = presence_by_key(&conn, machine_id, workspace_id)?;
        let Some(presence) = presence else {
            return Ok(SanitizedWorkspacePresence {
                workspace_id: workspace_id.to_string(),
                machine_id: machine_id.to_string(),
                owner_machine_id,
                freshness: ProjectionFreshness::Missing,
                last_verified: None,
                age_seconds: None,
            });
        };
        let age = age_seconds(now, presence.last_verified);
        let freshness = if now.signed_duration_since(presence.last_verified) > freshness_ttl {
            ProjectionFreshness::Stale
        } else {
            ProjectionFreshness::Current
        };
        Ok(SanitizedWorkspacePresence {
            workspace_id: workspace_id.to_string(),
            machine_id: machine_id.to_string(),
            owner_machine_id,
            freshness,
            last_verified: Some(presence.last_verified),
            age_seconds: Some(age),
        })
    }

    /// Authenticate and compare-and-set an owner profile. The caller supplies
    /// the generation it read (zero for missing). Semantically unchanged
    /// payloads keep the generation while refreshing owner/hub observations.
    pub fn publish_execution_profile(
        &self,
        caller_machine_id: &str,
        expected_generation: u64,
        profile: &ExecutionProfileV1,
        received_at: DateTime<Utc>,
        max_observation_age: Duration,
        max_future_skew: Duration,
    ) -> Result<StoredExecutionProfile, OrbitError> {
        validate_machine_id(caller_machine_id)?;
        validate_nonnegative_duration("maximum observation age", max_observation_age)?;
        validate_nonnegative_duration("maximum future skew", max_future_skew)?;
        profile.validate()?;
        if profile.owner_machine_id != caller_machine_id {
            return Err(OrbitError::InvalidInput(format!(
                "execution profile owner_machine_id '{}' does not match authenticated caller '{}'",
                profile.owner_machine_id, caller_machine_id
            )));
        }
        if profile.observed_at < received_at - max_observation_age {
            return Err(OrbitError::InvalidInput(format!(
                "execution profile observation for workspace_id '{}' is already stale",
                profile.workspace_id
            )));
        }
        if profile.observed_at > received_at + max_future_skew {
            return Err(OrbitError::InvalidInput(format!(
                "execution profile observation for workspace_id '{}' is implausibly future-dated",
                profile.workspace_id
            )));
        }

        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            require_active_host(&tx.tx, caller_machine_id)?;
            let ownership = ownership_by_workspace(&tx.tx, &profile.workspace_id)?.ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "workspace_id '{}' has no declared owner",
                    profile.workspace_id
                ))
            })?;
            if ownership.owner_machine_id != caller_machine_id {
                return Err(OrbitError::InvalidInput(format!(
                    "machine_id '{caller_machine_id}' is not the owner of workspace_id '{}'",
                    profile.workspace_id
                )));
            }

            let existing = stored_profile_by_workspace(&tx.tx, &profile.workspace_id)?;
            let actual_generation = existing.as_ref().map_or(0, |record| record.generation);
            if expected_generation != actual_generation {
                return Err(OrbitError::InvalidInput(format!(
                    "stale execution profile generation for workspace_id '{}': expected {}, current {}",
                    profile.workspace_id, expected_generation, actual_generation
                )));
            }
            if existing
                .as_ref()
                .is_some_and(|record| profile.observed_at < record.profile.observed_at)
            {
                return Err(OrbitError::InvalidInput(format!(
                    "execution profile observation for workspace_id '{}' is older than the stored observation",
                    profile.workspace_id
                )));
            }

            let generation = match existing.as_ref() {
                None => 1,
                Some(record) if record.profile.semantic_eq(profile) => record.generation,
                Some(record) => record.generation.checked_add(1).ok_or_else(|| {
                    OrbitError::Store(format!(
                        "execution profile generation overflow for workspace_id '{}'",
                        profile.workspace_id
                    ))
                })?,
            };
            let payload_json = serde_json::to_string(profile).map_err(|error| {
                OrbitError::Store(format!("serialize execution profile: {error}"))
            })?;
            let generation_sql = i64::try_from(generation).map_err(|error| {
                OrbitError::Store(format!("execution profile generation is too large: {error}"))
            })?;
            tx.tx
                .execute(
                    "INSERT INTO workspace_execution_profiles(
                        workspace_id, owner_machine_id, generation, payload_json, received_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(workspace_id) DO UPDATE SET
                        owner_machine_id = excluded.owner_machine_id,
                        generation = excluded.generation,
                        payload_json = excluded.payload_json,
                        received_at = excluded.received_at",
                    params![
                        profile.workspace_id,
                        caller_machine_id,
                        generation_sql,
                        payload_json,
                        received_at.to_rfc3339(),
                    ],
                )
                .map_err(|error| OrbitError::Store(format!(
                    "publish execution profile for workspace_id '{}': {error}",
                    profile.workspace_id
                )))?;
            // Profile publication always refreshes the hub receipt
            // (`received_at`), a freshness-visible change, so the global
            // revision advances even when the semantic generation is retained.
            advance_registry_revision(&tx.tx)?;
            stored_profile_by_workspace(&tx.tx, &profile.workspace_id)?.ok_or_else(|| {
                OrbitError::Store(format!(
                    "published execution profile for workspace_id '{}' but could not read it back",
                    profile.workspace_id
                ))
            })
        })
    }

    pub fn get_execution_profile(
        &self,
        workspace_id: &str,
    ) -> Result<Option<StoredExecutionProfile>, OrbitError> {
        validate_registry_text("workspace_id", workspace_id)?;
        let conn = self.read()?;
        stored_profile_by_workspace(&conn, workspace_id)
    }

    pub fn sanitized_execution_profile(
        &self,
        workspace_id: &str,
        now: DateTime<Utc>,
        freshness_ttl: Duration,
    ) -> Result<SanitizedExecutionProfile, OrbitError> {
        validate_registry_text("workspace_id", workspace_id)?;
        validate_nonnegative_duration("execution profile freshness TTL", freshness_ttl)?;
        let conn = self.read()?;
        let owner_machine_id = ownership_by_workspace(&conn, workspace_id)?
            .map(|ownership| ownership.owner_machine_id);
        let record = stored_profile_by_workspace(&conn, workspace_id)?;
        let Some(record) = record else {
            return Ok(SanitizedExecutionProfile {
                workspace_id: workspace_id.to_string(),
                owner_machine_id,
                freshness: ProjectionFreshness::Missing,
                generation: None,
                observed_at: None,
                received_at: None,
                age_seconds: None,
                profile: None,
            });
        };
        let age = age_seconds(now, record.received_at);
        let freshness = if now.signed_duration_since(record.received_at) > freshness_ttl {
            ProjectionFreshness::Stale
        } else {
            ProjectionFreshness::Current
        };
        Ok(SanitizedExecutionProfile {
            workspace_id: workspace_id.to_string(),
            owner_machine_id,
            freshness,
            generation: Some(record.generation),
            observed_at: Some(record.profile.observed_at),
            received_at: Some(record.received_at),
            age_seconds: Some(age),
            profile: Some(record.profile),
        })
    }
}

/// Advance the hub-global registry revision by exactly one inside the caller's
/// write transaction. Callers invoke this only on the snapshot-visible mutation
/// branch, never on an idempotent no-op return.
pub(crate) fn advance_registry_revision(conn: &Connection) -> Result<(), OrbitError> {
    let updated = conn
        .execute(
            "UPDATE hub_registry_metadata
             SET registry_revision = registry_revision + 1, updated_at = ?1
             WHERE id = 0",
            params![crate::now_string()],
        )
        .map_err(|error| OrbitError::Store(format!("advance registry revision: {error}")))?;
    if updated != 1 {
        return Err(OrbitError::Store(
            "hub registry metadata singleton row is missing".to_string(),
        ));
    }
    Ok(())
}

fn require_active_host(conn: &Connection, machine_id: &str) -> Result<HostRecord, OrbitError> {
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

fn ownership_by_workspace(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Option<WorkspaceOwnership>, OrbitError> {
    conn.query_row(
        "SELECT workspace_id, owner_machine_id, bound_at, updated_at
         FROM workspace_ownership WHERE workspace_id = ?1",
        [workspace_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(|(workspace_id, owner_machine_id, bound_at, updated_at)| {
        Ok(WorkspaceOwnership {
            workspace_id,
            owner_machine_id,
            bound_at: crate::parse_timestamp(&bound_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            updated_at: crate::parse_timestamp(&updated_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
        })
    })
    .transpose()
}

fn presence_for_machine(
    conn: &Connection,
    machine_id: &str,
) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
    let mut statement = conn
        .prepare(
            "SELECT machine_id, workspace_id, root, last_verified
             FROM host_workspace_presence WHERE machine_id = ?1 ORDER BY workspace_id",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([machine_id], raw_presence_row)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    rows.map(|row| {
        row.map_err(|error| OrbitError::Store(error.to_string()))?
            .try_into()
    })
    .collect()
}

fn presence_by_key(
    conn: &Connection,
    machine_id: &str,
    workspace_id: &str,
) -> Result<Option<HostWorkspacePresence>, OrbitError> {
    conn.query_row(
        "SELECT machine_id, workspace_id, root, last_verified
         FROM host_workspace_presence WHERE machine_id = ?1 AND workspace_id = ?2",
        params![machine_id, workspace_id],
        raw_presence_row,
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(TryInto::try_into)
    .transpose()
}

struct RawPresenceRow {
    machine_id: String,
    workspace_id: String,
    root: String,
    last_verified: String,
}

fn raw_presence_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawPresenceRow> {
    Ok(RawPresenceRow {
        machine_id: row.get(0)?,
        workspace_id: row.get(1)?,
        root: row.get(2)?,
        last_verified: row.get(3)?,
    })
}

impl TryFrom<RawPresenceRow> for HostWorkspacePresence {
    type Error = OrbitError;

    fn try_from(row: RawPresenceRow) -> Result<Self, Self::Error> {
        Ok(Self {
            machine_id: row.machine_id,
            workspace_id: row.workspace_id,
            root: row.root.into(),
            last_verified: crate::parse_timestamp(&row.last_verified)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
        })
    }
}

fn stored_profile_by_workspace(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Option<StoredExecutionProfile>, OrbitError> {
    conn.query_row(
        "SELECT payload_json, generation, received_at
         FROM workspace_execution_profiles WHERE workspace_id = ?1",
        [workspace_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))?
    .map(|(payload_json, generation, received_at)| {
        let profile =
            serde_json::from_str::<ExecutionProfileV1>(&payload_json).map_err(|error| {
                OrbitError::Store(format!(
                    "workspace_id '{workspace_id}' has invalid execution profile JSON: {error}"
                ))
            })?;
        profile.validate().map_err(|error| {
            OrbitError::Store(format!(
                "workspace_id '{workspace_id}' has invalid execution profile: {error}"
            ))
        })?;
        let generation = u64::try_from(generation).map_err(|error| {
            OrbitError::Store(format!(
                "workspace_id '{workspace_id}' has invalid profile generation: {error}"
            ))
        })?;
        Ok(StoredExecutionProfile {
            profile,
            generation,
            received_at: crate::parse_timestamp(&received_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
        })
    })
    .transpose()
}

fn validate_presence_root(root: &Path) -> Result<(), OrbitError> {
    if !root.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "presence root '{}' must be absolute",
            root.display()
        )));
    }
    let raw = root
        .to_str()
        .ok_or_else(|| OrbitError::InvalidInput("presence root must be valid UTF-8".to_string()))?;
    if raw.chars().any(char::is_control) {
        return Err(OrbitError::InvalidInput(
            "presence root must not contain control characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_nonnegative_duration(field: &str, duration: Duration) -> Result<(), OrbitError> {
    if duration < Duration::zero() {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}

fn age_seconds(now: DateTime<Utc>, observed: DateTime<Utc>) -> u64 {
    u64::try_from(now.signed_duration_since(observed).num_seconds().max(0)).unwrap_or_default()
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

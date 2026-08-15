//! Workspace ownership, presence, and execution-profile persistence.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    ExecutionProfileV1, HostWorkspacePresence, OrbitError, ProjectionFreshness,
    SanitizedExecutionProfile, SanitizedWorkspacePresence, StoredExecutionProfile,
    WorkspaceOwnership, WorkspacePresenceDeclaration, validate_machine_id,
    validate_registry_identifier,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::super::RegistryStore;
use super::advance_registry_revision;
use super::hosts::require_active_host;

impl RegistryStore {
    /// Declare the one durable owner of an existing logical workspace. The
    /// service layer validates logical-workspace existence; this store layer
    /// validates stable identifiers and active host lifecycle. Repeating the
    /// same binding is idempotent, while rebinding fails closed.
    pub fn bind_workspace_owner(
        &self,
        workspace_id: &str,
        owner_machine_id: &str,
    ) -> Result<WorkspaceOwnership, OrbitError> {
        validate_registry_identifier("workspace_id", workspace_id)?;
        validate_machine_id(owner_machine_id)?;
        self.store.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            require_active_host(tx.connection(), owner_machine_id)?;
            if let Some(existing) = ownership_by_workspace(tx.connection(), workspace_id)? {
                if existing.owner_machine_id != owner_machine_id {
                    return Err(OrbitError::InvalidInput(format!(
                        "workspace_id '{workspace_id}' is already owned by machine_id '{}'; ownership migration must be explicit",
                        existing.owner_machine_id
                    )));
                }
                return Ok(existing);
            }
            let now = super::super::now_string();
            tx.connection()
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
            advance_registry_revision(tx.connection())?;
            ownership_by_workspace(tx.connection(), workspace_id)?.ok_or_else(|| {
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
        validate_registry_identifier("workspace_id", workspace_id)?;
        self.read(|conn| ownership_by_workspace(conn, workspace_id))
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
            validate_registry_identifier("workspace_id", &declaration.workspace_id)?;
            if !workspace_ids.insert(declaration.workspace_id.as_str()) {
                return Err(OrbitError::InvalidInput(format!(
                    "presence declaration repeats workspace_id '{}'",
                    declaration.workspace_id
                )));
            }
            validate_presence_root(&declaration.root)?;
        }

        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let host = require_active_host(tx.connection(), caller_machine_id)?;
                let mut intended = declarations
                    .iter()
                    .map(|declaration| HostWorkspacePresence {
                        machine_id: caller_machine_id.to_string(),
                        workspace_id: declaration.workspace_id.clone(),
                        root: declaration.root.clone(),
                        last_verified: declaration.last_verified,
                    })
                    .collect::<Vec<_>>();
                intended.sort_by(|left, right| left.workspace_id.cmp(&right.workspace_id));
                let current = presence_for_machine(tx.connection(), caller_machine_id)?;
                if host.last_seen_at == Some(received_at) && current == intended {
                    return Ok(current);
                }

                tx.connection()
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
                    tx.connection()
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
                tx.connection()
                    .execute(
                        "UPDATE hosts SET last_seen_at = ?2 WHERE machine_id = ?1",
                        params![caller_machine_id, received_at.to_rfc3339()],
                    )
                    .map_err(|error| {
                        OrbitError::Store(format!("update host last_seen: {error}"))
                    })?;
                // A changed declaration or receipt (`last_seen_at` /
                // `last_verified`) is freshness-visible. An exact replay returned
                // above without rewriting rows or advancing the revision.
                advance_registry_revision(tx.connection())?;
                presence_for_machine(tx.connection(), caller_machine_id)
            })
    }

    pub fn list_host_workspace_presence_private(
        &self,
        machine_id: &str,
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        validate_machine_id(machine_id)?;
        self.read(|conn| presence_for_machine(conn, machine_id))
    }

    pub fn sanitized_workspace_presence(
        &self,
        machine_id: &str,
        workspace_id: &str,
        now: DateTime<Utc>,
        freshness_ttl: Duration,
    ) -> Result<SanitizedWorkspacePresence, OrbitError> {
        validate_machine_id(machine_id)?;
        validate_registry_identifier("workspace_id", workspace_id)?;
        validate_nonnegative_duration("presence freshness TTL", freshness_ttl)?;
        self.read(|conn| {
            let owner_machine_id = ownership_by_workspace(conn, workspace_id)?
                .map(|ownership| ownership.owner_machine_id);
            let presence = presence_by_key(conn, machine_id, workspace_id)?;
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

        self.store.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            require_active_host(tx.connection(), caller_machine_id)?;
            let ownership = ownership_by_workspace(tx.connection(), &profile.workspace_id)?.ok_or_else(|| {
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

            let existing = stored_profile_by_workspace(tx.connection(), &profile.workspace_id)?;
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
            if let Some(existing) = existing.as_ref()
                && existing.profile == *profile
                && existing.received_at == received_at
            {
                return Ok(existing.clone());
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
            tx.connection()
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
            // A changed profile observation or hub receipt (`received_at`) is
            // freshness-visible, so it advances the global revision even when
            // semantic generation is retained. An exact replay returned above.
            advance_registry_revision(tx.connection())?;
            stored_profile_by_workspace(tx.connection(), &profile.workspace_id)?.ok_or_else(|| {
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
        validate_registry_identifier("workspace_id", workspace_id)?;
        self.read(|conn| stored_profile_by_workspace(conn, workspace_id))
    }

    pub fn sanitized_execution_profile(
        &self,
        workspace_id: &str,
        now: DateTime<Utc>,
        freshness_ttl: Duration,
    ) -> Result<SanitizedExecutionProfile, OrbitError> {
        validate_registry_identifier("workspace_id", workspace_id)?;
        validate_nonnegative_duration("execution profile freshness TTL", freshness_ttl)?;
        self.read(|conn| {
            let owner_machine_id = ownership_by_workspace(conn, workspace_id)?
                .map(|ownership| ownership.owner_machine_id);
            let record = stored_profile_by_workspace(conn, workspace_id)?;
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
        })
    }
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
            bound_at: super::super::parse_timestamp(&bound_at)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            updated_at: super::super::parse_timestamp(&updated_at)
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
            last_verified: super::super::parse_timestamp(&row.last_verified)
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
            received_at: super::super::parse_timestamp(&received_at)
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

//! Hub-global registry metadata and the single-transaction sanitized registry
//! snapshot read [ORB-10267].
//!
//! The snapshot is the one typed, path-free projection consumed by the
//! `orbit.host.list` / `orbit.workspace.list` discovery tools and by the
//! satellite registry cache. It is read with the hub `machine_id` and the
//! hub-global `registry_revision` inside one read transaction so a concurrent
//! mutation can never tear identity, revision, and content apart.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    HostStatus, OrbitError, ProjectionFreshness, REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistryAliasV1,
    RegistryHostV1, RegistryPresenceV1, RegistryProfileV1, RegistrySnapshotV1, RegistryWorkspaceV1,
};
use rusqlite::{Connection, OptionalExtension};

use super::RemoteStore;

impl RemoteStore {
    /// The configured hub `machine_id`, or `None` when the hub identity has not
    /// been stamped yet.
    pub fn hub_machine_id(&self) -> Result<Option<String>, OrbitError> {
        self.read(hub_machine_id)
    }

    /// The current hub-global registry revision.
    pub fn registry_revision(&self) -> Result<u64, OrbitError> {
        self.read(registry_revision)
    }

    /// Read the whole sanitized registry snapshot — hub identity, revision,
    /// hosts (active and retired) with aliases and presence freshness, and
    /// owned workspaces with execution-profile freshness — in one read
    /// transaction. No presence root, checkout path, or raw execution-profile
    /// payload is read; only the allowlisted sanitized fields are.
    pub fn read_registry_snapshot(
        &self,
        now: DateTime<Utc>,
        presence_freshness_ttl: Duration,
        profile_freshness_ttl: Duration,
    ) -> Result<RegistrySnapshotV1, OrbitError> {
        self.store.with_transaction(|tx| {
            read_snapshot(
                tx.connection(),
                now,
                presence_freshness_ttl,
                profile_freshness_ttl,
            )
        })
    }
}

fn hub_machine_id(conn: &Connection) -> Result<Option<String>, OrbitError> {
    conn.query_row(
        "SELECT hub_machine_id FROM hub_registry_metadata WHERE id = 0",
        [],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))
    .map(Option::flatten)
}

fn registry_revision(conn: &Connection) -> Result<u64, OrbitError> {
    let revision: i64 = conn
        .query_row(
            "SELECT registry_revision FROM hub_registry_metadata WHERE id = 0",
            [],
            |row| row.get(0),
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    u64::try_from(revision)
        .map_err(|error| OrbitError::Store(format!("registry revision is negative: {error}")))
}

fn read_snapshot(
    conn: &Connection,
    now: DateTime<Utc>,
    presence_freshness_ttl: Duration,
    profile_freshness_ttl: Duration,
) -> Result<RegistrySnapshotV1, OrbitError> {
    let hub_machine_id = hub_machine_id(conn)?;
    let registry_revision = registry_revision(conn)?;

    let mut aliases = read_aliases_by_machine(conn)?;
    let mut presence = read_presence_by_machine(conn, now, presence_freshness_ttl)?;
    let hosts = read_hosts(conn)?
        .into_iter()
        .map(|host| RegistryHostV1 {
            aliases: aliases.remove(&host.machine_id).unwrap_or_default(),
            presence: presence.remove(&host.machine_id).unwrap_or_default(),
            machine_id: host.machine_id,
            host_id: host.host_id,
            labels: host.labels,
            status: host.status,
            registered_at: host.registered_at,
            updated_at: host.updated_at,
            retired_at: host.retired_at,
            last_seen_at: host.last_seen_at,
        })
        .collect();

    let workspaces = read_workspaces(conn, now, profile_freshness_ttl)?;

    Ok(RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id,
        registry_revision,
        hosts,
        workspaces,
    })
}

struct RawHost {
    machine_id: String,
    host_id: String,
    labels: BTreeSet<String>,
    status: HostStatus,
    registered_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    retired_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
}

fn read_hosts(conn: &Connection) -> Result<Vec<RawHost>, OrbitError> {
    let mut statement = conn
        .prepare(
            "SELECT machine_id, host_id, labels_json, status, registered_at,
                    updated_at, retired_at, last_seen_at
             FROM hosts ORDER BY host_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut hosts = Vec::new();
    for row in rows {
        let (
            machine_id,
            host_id,
            labels_json,
            status,
            registered_at,
            updated_at,
            retired_at,
            last_seen_at,
        ) = row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let labels: BTreeSet<String> = serde_json::from_str(&labels_json).map_err(|error| {
            OrbitError::Store(format!(
                "host_id '{host_id}' has invalid labels_json: {error}"
            ))
        })?;
        hosts.push(RawHost {
            status: HostStatus::from_str(&status)
                .map_err(|error| OrbitError::Store(format!("host_id '{host_id}': {error}")))?,
            registered_at: parse_ts(&registered_at)?,
            updated_at: parse_ts(&updated_at)?,
            retired_at: parse_opt_ts(retired_at)?,
            last_seen_at: parse_opt_ts(last_seen_at)?,
            machine_id,
            host_id,
            labels,
        });
    }
    Ok(hosts)
}

fn read_aliases_by_machine(
    conn: &Connection,
) -> Result<BTreeMap<String, Vec<RegistryAliasV1>>, OrbitError> {
    let mut statement = conn
        .prepare(
            "SELECT machine_id, host_id, created_at, warning
             FROM host_aliases ORDER BY machine_id ASC, created_at ASC, host_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut by_machine: BTreeMap<String, Vec<RegistryAliasV1>> = BTreeMap::new();
    for row in rows {
        let (machine_id, alias_host_id, created_at, warning) =
            row.map_err(|error| OrbitError::Store(error.to_string()))?;
        by_machine
            .entry(machine_id)
            .or_default()
            .push(RegistryAliasV1 {
                alias_host_id,
                created_at: parse_ts(&created_at)?,
                warning,
            });
    }
    Ok(by_machine)
}

fn read_presence_by_machine(
    conn: &Connection,
    now: DateTime<Utc>,
    freshness_ttl: Duration,
) -> Result<BTreeMap<String, Vec<RegistryPresenceV1>>, OrbitError> {
    // `root` is deliberately not selected: it never enters the snapshot.
    let mut statement = conn
        .prepare(
            "SELECT machine_id, workspace_id, last_verified
             FROM host_workspace_presence ORDER BY machine_id ASC, workspace_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut by_machine: BTreeMap<String, Vec<RegistryPresenceV1>> = BTreeMap::new();
    for row in rows {
        let (machine_id, workspace_id, last_verified) =
            row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let last_verified = parse_ts(&last_verified)?;
        let (freshness, age) = freshness_and_age(now, last_verified, freshness_ttl);
        by_machine
            .entry(machine_id)
            .or_default()
            .push(RegistryPresenceV1 {
                workspace_id,
                freshness,
                last_verified: Some(last_verified),
                age_seconds: Some(age),
            });
    }
    Ok(by_machine)
}

fn read_workspaces(
    conn: &Connection,
    now: DateTime<Utc>,
    freshness_ttl: Duration,
) -> Result<Vec<RegistryWorkspaceV1>, OrbitError> {
    // `payload_json` is deliberately not selected: no crew/model content ever
    // enters the snapshot. Owner display names come from the hosts join.
    let mut statement = conn
        .prepare(
            "SELECT o.workspace_id, o.owner_machine_id, h.host_id,
                    p.generation, p.received_at, p.observed_at
             FROM workspace_ownership o
             LEFT JOIN hosts h ON h.machine_id = o.owner_machine_id
             LEFT JOIN (
                 SELECT wep.workspace_id, wep.generation, wep.received_at,
                        json_extract(wep.payload_json, '$.observed_at') AS observed_at
                 FROM workspace_execution_profiles wep
             ) p ON p.workspace_id = o.workspace_id
             ORDER BY o.workspace_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut workspaces = Vec::new();
    for row in rows {
        let (workspace_id, owner_machine_id, owner_host_id, generation, received_at, observed_at) =
            row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let profile = match (generation, received_at.as_deref()) {
            (Some(generation), Some(received_at)) => {
                let generation = u64::try_from(generation).map_err(|error| {
                    OrbitError::Store(format!(
                        "workspace_id '{workspace_id}' has invalid profile generation: {error}"
                    ))
                })?;
                let received = parse_ts(received_at)?;
                let observed = observed_at.as_deref().map(parse_ts).transpose()?;
                let (freshness, age) = freshness_and_age(now, received, freshness_ttl);
                RegistryProfileV1 {
                    freshness,
                    generation: Some(generation),
                    observed_at: observed,
                    received_at: Some(received),
                    age_seconds: Some(age),
                }
            }
            _ => RegistryProfileV1 {
                freshness: ProjectionFreshness::Missing,
                generation: None,
                observed_at: None,
                received_at: None,
                age_seconds: None,
            },
        };
        workspaces.push(RegistryWorkspaceV1 {
            workspace_id,
            owner_machine_id,
            owner_host_id,
            profile,
        });
    }
    Ok(workspaces)
}

fn freshness_and_age(
    now: DateTime<Utc>,
    observed: DateTime<Utc>,
    freshness_ttl: Duration,
) -> (ProjectionFreshness, u64) {
    let age =
        u64::try_from(now.signed_duration_since(observed).num_seconds().max(0)).unwrap_or_default();
    let freshness = if now.signed_duration_since(observed) > freshness_ttl {
        ProjectionFreshness::Stale
    } else {
        ProjectionFreshness::Current
    };
    (freshness, age)
}

fn parse_ts(value: &str) -> Result<DateTime<Utc>, OrbitError> {
    super::parse_timestamp(value).map_err(|error| OrbitError::Store(error.to_string()))
}

fn parse_opt_ts(value: Option<String>) -> Result<Option<DateTime<Utc>>, OrbitError> {
    value.as_deref().map(parse_ts).transpose()
}

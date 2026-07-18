//! Path-free, sanitized hub registry snapshot and satellite registry cache
//! codec [ORB-10267].
//!
//! [`RegistrySnapshotV1`] is the single typed projection read from the hub
//! coordination store in one transaction. It is the sole input to the
//! `orbit.host.list` / `orbit.workspace.list` discovery tools and to the
//! satellite [`RegistryCacheV1`] serialization. It deliberately carries no
//! presence root, checkout/worktree path, raw execution-profile payload,
//! crew/model identity, secret, credential, SSH configuration, or repository
//! content: every field below is an explicit allowlisted stable identity,
//! lifecycle marker, or sanitized freshness value.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::execution_profile::ProjectionFreshness;
use crate::types::host::HostStatus;

/// Schema version of [`RegistrySnapshotV1`]. Bumped only for a
/// backward-incompatible projection change.
pub const REGISTRY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Schema version of the on-disk [`RegistryCacheV1`]. A cache written by a
/// newer binary (higher version) is treated as unsupported-future input and is
/// never rewritten by an older binary.
pub const REGISTRY_CACHE_SCHEMA_VERSION: u32 = 1;

/// One sanitized, path-free projection of the hub registry, read with the hub
/// `machine_id` and hub-global `registry_revision` in a single store read
/// transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshotV1 {
    pub schema_version: u32,
    /// The configured hub `machine_id`, or `None` when the hub identity has
    /// not yet been stamped into the registry-metadata singleton.
    pub hub_machine_id: Option<String>,
    /// Hub-global monotonic revision advanced once per snapshot-visible
    /// mutation.
    pub registry_revision: u64,
    pub hosts: Vec<RegistryHostV1>,
    pub workspaces: Vec<RegistryWorkspaceV1>,
}

impl RegistrySnapshotV1 {
    /// Compare the canonical hub payload (identity, revision, and sanitized
    /// content) independently of any locally stamped receipt time. Two
    /// snapshots with the same payload may renew a cache receipt without a
    /// rewrite; a different payload at equal revision must be rejected.
    pub fn canonical_payload_eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.hub_machine_id == other.hub_machine_id
            && self.registry_revision == other.registry_revision
            && self.hosts == other.hosts
            && self.workspaces == other.workspaces
    }
}

/// Sanitized per-host projection: stable machine identity, current display
/// name, labels, lifecycle/liveness, permanent aliases, and workspace-presence
/// identity/freshness. No presence root ever appears here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryHostV1 {
    pub machine_id: String,
    pub host_id: String,
    pub labels: BTreeSet<String>,
    pub status: HostStatus,
    pub registered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub aliases: Vec<RegistryAliasV1>,
    pub presence: Vec<RegistryPresenceV1>,
}

/// A permanent tombstone alias with its warning metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAliasV1 {
    pub alias_host_id: String,
    pub created_at: DateTime<Utc>,
    pub warning: String,
}

/// Sanitized workspace-presence marker: workspace identity plus freshness,
/// never the advertised checkout root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryPresenceV1 {
    pub workspace_id: String,
    pub freshness: ProjectionFreshness,
    pub last_verified: Option<DateTime<Utc>>,
    pub age_seconds: Option<u64>,
}

/// Sanitized per-workspace projection: stable workspace identity, declared
/// owner identity/display name, and sanitized execution-profile freshness.
/// Never carries the raw execution-profile payload, crews, or models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryWorkspaceV1 {
    pub workspace_id: String,
    pub owner_machine_id: String,
    /// The owner's current display name, resolved from the host record, or
    /// `None` when the owner host is absent.
    pub owner_host_id: Option<String>,
    pub profile: RegistryProfileV1,
}

/// Allowlisted execution-profile freshness/generation/age. Built explicitly
/// from the sanitized projection; the internal `SanitizedExecutionProfile`
/// (which embeds the raw `ExecutionProfileV1` with crews and models) is never
/// serialized here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryProfileV1 {
    pub freshness: ProjectionFreshness,
    pub generation: Option<u64>,
    pub observed_at: Option<DateTime<Utc>>,
    pub received_at: Option<DateTime<Utc>>,
    pub age_seconds: Option<u64>,
}

/// The versioned satellite registry cache: one sanitized hub snapshot plus the
/// local receipt time at which this machine accepted it. The snapshot is the
/// canonical payload; `received_at` is the locally stamped receipt used for
/// age computation and is compared separately from the payload on refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryCacheV1 {
    pub schema_version: u32,
    /// Local wall-clock receipt time. Age is always computed from this local
    /// stamp, never from a remote clock.
    pub received_at: DateTime<Utc>,
    pub snapshot: RegistrySnapshotV1,
}

#[cfg(test)]
#[path = "tests/registry_snapshot.rs"]
mod tests;

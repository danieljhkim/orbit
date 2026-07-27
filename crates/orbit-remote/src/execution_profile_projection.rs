//! Hub-side projection-backed crew discovery and explicit task-crew validation
//! [ORB-10276].
//!
//! This is the single reusable service that both `orbit.crew.list` discovery
//! and explicit task-crew validation read through. It consumes C2's stored
//! owner execution-profile projection (never hub-local crews, the satellite
//! registry cache, a stale replica, or a synchronous owner call), applies one
//! service-owned freshness TTL through an injected clock, and returns either a
//! sanitized [`CrewDiscoveryV1`] or an immutable [`ValidatedCrewProfile`].
//!
//! Discovery and validation observe the exact same workspace-specific profile
//! generation and freshness because they share this one read path: a newer
//! semantic profile advances the generation for both, and an identically
//! refreshed profile updates freshness without inventing a generation.

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    CREW_DISCOVERY_SCHEMA_VERSION, CrewDiscoveryV1, OrbitError, ProjectionFreshness,
    RegistryProfileV1, SanitizedExecutionProfile, ValidatedCrewProfile,
};

use crate::persistence::RemoteStore;
use crate::service::remote_store_at;

/// The single service-owned execution-profile freshness TTL shared by crew
/// discovery and task-crew validation. It matches C2's profile-status TTL so a
/// workspace never reports one freshness through discovery and a different one
/// through validation.
pub const CREW_PROFILE_FRESHNESS_TTL: Duration = Duration::minutes(10);

/// Injected, testable wall clock for freshness computation. Production uses
/// [`SystemProfileClock`]; deterministic fixtures inject a fixed clock so
/// missing/current/stale transitions are exercised without sleeping.
pub trait ProfileClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// The real wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProfileClock;

impl ProfileClock for SystemProfileClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Fixed clock for deterministic tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedProfileClock(pub DateTime<Utc>);

impl ProfileClock for FixedProfileClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Projection-backed crew discovery and validation over one owner
/// execution-profile store.
#[derive(Clone)]
pub struct ExecutionProfileProjection {
    store: RemoteStore,
    clock: Arc<dyn ProfileClock>,
    freshness_ttl: Duration,
}

impl ExecutionProfileProjection {
    /// Build over an already-opened Remote store using the system clock and the
    /// shared freshness TTL.
    pub fn new(store: RemoteStore) -> Self {
        Self {
            store,
            clock: Arc::new(SystemProfileClock),
            freshness_ttl: CREW_PROFILE_FRESHNESS_TTL,
        }
    }

    /// Build over the config-resolved coordination store for a machine-global
    /// root.
    pub fn at(global_root: &Path) -> Result<Self, OrbitError> {
        Ok(Self::new(remote_store_at(global_root)?))
    }

    /// Test seam: inject a deterministic clock (and TTL).
    pub fn with_clock(
        store: RemoteStore,
        clock: Arc<dyn ProfileClock>,
        freshness_ttl: Duration,
    ) -> Self {
        Self {
            store,
            clock,
            freshness_ttl,
        }
    }

    fn status(&self, workspace_id: &str) -> Result<SanitizedExecutionProfile, OrbitError> {
        self.store
            .sanitized_execution_profile(workspace_id, self.clock.now(), self.freshness_ttl)
    }

    /// Sanitized `orbit.crew.list` projection for one workspace. Missing and
    /// stale profiles remain inspectable discovery states; crews returned from a
    /// stale record stay bound to that stale generation via the shared profile
    /// envelope and carry no dispatch-eligible marker.
    pub fn crew_discovery(&self, workspace_id: &str) -> Result<CrewDiscoveryV1, OrbitError> {
        Ok(build_crew_discovery(
            workspace_id,
            self.status(workspace_id)?,
        ))
    }

    /// Validate an explicit, non-empty task crew against the current owner
    /// profile. A current profile whose effective crews contain `crew` yields
    /// the immutable [`ValidatedCrewProfile`]; a missing profile, a stale
    /// profile, or an unknown crew fails with an actionable
    /// workspace/owner/state/age error and never falls back to any other crew
    /// source.
    pub fn validate_task_crew(
        &self,
        workspace_id: &str,
        crew: &str,
    ) -> Result<ValidatedCrewProfile, OrbitError> {
        let status = self.status(workspace_id)?;
        let owner = status.owner_machine_id.clone().unwrap_or_default();
        match status.freshness {
            ProjectionFreshness::Missing => Err(OrbitError::InvalidInput(format!(
                "cannot validate crew '{crew}' for workspace '{workspace_id}': owner '{}' has no published execution profile (state=missing). File the task without a crew, or clear it, until the owner publishes a profile",
                owner_label(&owner)
            ))),
            ProjectionFreshness::Stale => Err(OrbitError::InvalidInput(format!(
                "cannot validate crew '{crew}' for workspace '{workspace_id}': owner '{}' execution profile is stale (state=stale{}). Refuse dispatch-affecting crew assignment until the owner republishes",
                owner_label(&owner),
                age_suffix(status.age_seconds),
            ))),
            ProjectionFreshness::Current => {
                let profile = status.profile.ok_or_else(|| {
                    OrbitError::Store(format!(
                        "workspace '{workspace_id}' reports a current execution profile without a payload"
                    ))
                })?;
                let resolved_crew = profile
                    .crews
                    .iter()
                    .find(|entry| entry.name == crew)
                    .cloned()
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(format!(
                            "crew '{crew}' is not published in owner '{}' execution profile for workspace '{workspace_id}' (state=current, generation={}); known crews: [{}]",
                            owner_label(&owner),
                            status.generation.unwrap_or_default(),
                            profile
                                .crews
                                .iter()
                                .map(|entry| entry.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    })?;
                Ok(ValidatedCrewProfile {
                    workspace_id: workspace_id.to_string(),
                    owner_machine_id: profile.owner_machine_id.clone(),
                    generation: status.generation.unwrap_or_default(),
                    config_digest: profile.config_digest.clone(),
                    ship_closure_digest: profile.ship.ship_closure_digest.clone(),
                    resolved_crew,
                    profile,
                })
            }
        }
    }
}

fn build_crew_discovery(workspace_id: &str, status: SanitizedExecutionProfile) -> CrewDiscoveryV1 {
    let profile = RegistryProfileV1 {
        freshness: status.freshness,
        generation: status.generation,
        observed_at: status.observed_at,
        received_at: status.received_at,
        age_seconds: status.age_seconds,
    };
    let (default_crew, crews) = match status.profile {
        Some(profile) => (Some(profile.default_crew), profile.crews),
        None => (None, Vec::new()),
    };
    CrewDiscoveryV1 {
        schema_version: CREW_DISCOVERY_SCHEMA_VERSION,
        workspace_id: workspace_id.to_string(),
        owner_machine_id: status.owner_machine_id,
        profile,
        default_crew,
        crews,
    }
}

fn owner_label(owner: &str) -> &str {
    if owner.is_empty() { "unknown" } else { owner }
}

fn age_suffix(age_seconds: Option<u64>) -> String {
    match age_seconds {
        Some(age) => format!(", age={age}s"),
        None => String::new(),
    }
}

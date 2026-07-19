//! Registry-aware routine-pin validation [ORB-10270].
//!
//! Routine definitions remain loadable while a spoke is offline. This module
//! turns the already-classified registry cache into deterministic diagnostics
//! and an eligibility decision without opening a network connection or
//! mutating identity, cache, registry, routine, or scheduler state.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{HostStatus, OrbitError, RegistryHostV1, RegistrySnapshotV1};
use orbit_registry::{
    HostIdentity, HostMode, HostRegistryService, RegistryCacheService, RegistryCacheState,
};
use orbit_store::Store;
use serde::Serialize;

use super::loader::RoutineOrigin;

/// Default maximum age for the spoke's machine-local registry cache.
pub const DEFAULT_REGISTRY_CACHE_MAX_AGE_SECONDS: i64 = 5 * 60;
/// Default interval after which an otherwise active host is reported quiet.
pub const DEFAULT_QUIET_HOST_AFTER_SECONDS: i64 = 5 * 60;

/// Warning/error classification kept separate from routine load errors and
/// sweep infrastructure failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineDiagnosticSeverity {
    Warning,
    Error,
}

impl RoutineDiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// One stable cache or per-pin diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutineValidationDiagnostic {
    pub code: &'static str,
    pub severity: RoutineDiagnosticSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub message: String,
    /// True when the conclusion came from a decodable but stale cache.
    pub stale: bool,
}

/// Additive registry-source metadata shared by list/show/sweep output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutineRegistryStatus {
    pub source: &'static str,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<u64>,
    pub diagnostics: Vec<RoutineValidationDiagnostic>,
}

impl Default for RoutineRegistryStatus {
    fn default() -> Self {
        Self {
            source: "unavailable",
            state: "not_evaluated",
            age_seconds: None,
            diagnostics: Vec::new(),
        }
    }
}

/// Eligibility and diagnostics for one loaded routine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutinePinValidation {
    pub eligible: bool,
    pub diagnostics: Vec<RoutineValidationDiagnostic>,
}

/// Registry-neutral local identity consumed by routine placement logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineHostIdentity {
    pub machine_id: String,
    pub host_id: String,
}

/// Read-only identity surface consumed by the pure pin validator.
pub trait RoutineHostIdentityView {
    fn machine_id(&self) -> &str;
    fn host_id(&self) -> &str;
}

impl RoutineHostIdentityView for RoutineHostIdentity {
    fn machine_id(&self) -> &str {
        &self.machine_id
    }

    fn host_id(&self) -> &str {
        &self.host_id
    }
}

// Transitional adapter for the legacy Core-owned registry composition. The
// neutral identity above is the contract a future feature crate supplies.
impl RoutineHostIdentityView for HostIdentity {
    fn machine_id(&self) -> &str {
        &self.machine_id
    }

    fn host_id(&self) -> &str {
        &self.host_id
    }
}

impl From<&HostIdentity> for RoutineHostIdentity {
    fn from(identity: &HostIdentity) -> Self {
        Self {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
        }
    }
}

/// Registry-neutral spoke cache projection used by routine placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineRegistryCacheView {
    Current {
        snapshot: Box<RegistrySnapshotV1>,
        age_seconds: u64,
    },
    Stale {
        snapshot: Box<RegistrySnapshotV1>,
        age_seconds: u64,
    },
    Missing,
    Malformed {
        reason: String,
    },
    UnsupportedFuture {
        schema_version: u32,
    },
}

/// The local-only, registry-neutral input used by the pure pin validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineRegistryView {
    Standalone,
    Hub { snapshot: RegistrySnapshotV1 },
    Spoke { cache: RoutineRegistryCacheView },
}

impl RoutineRegistryView {
    /// Stable source/state metadata and cache-degradation diagnostics.
    pub fn status(&self) -> RoutineRegistryStatus {
        match self {
            Self::Standalone => RoutineRegistryStatus {
                source: "standalone",
                state: "current",
                age_seconds: None,
                diagnostics: Vec::new(),
            },
            Self::Hub { .. } => RoutineRegistryStatus {
                source: "hub",
                state: "current",
                age_seconds: None,
                diagnostics: Vec::new(),
            },
            Self::Spoke {
                cache: RoutineRegistryCacheView::Current { age_seconds, .. },
            } => RoutineRegistryStatus {
                source: "spoke_cache",
                state: "current",
                age_seconds: Some(*age_seconds),
                diagnostics: Vec::new(),
            },
            Self::Spoke {
                cache: RoutineRegistryCacheView::Stale { age_seconds, .. },
            } => RoutineRegistryStatus {
                source: "spoke_cache",
                state: "stale",
                age_seconds: Some(*age_seconds),
                diagnostics: vec![diagnostic(
                    "registry_cache_stale",
                    RoutineDiagnosticSeverity::Warning,
                    None,
                    format!("registry cache is stale (observed age {age_seconds}s)"),
                    true,
                )],
            },
            Self::Spoke {
                cache: RoutineRegistryCacheView::Missing,
            } => RoutineRegistryStatus {
                source: "spoke_cache",
                state: "missing",
                age_seconds: None,
                diagnostics: vec![diagnostic(
                    "registry_cache_missing",
                    RoutineDiagnosticSeverity::Warning,
                    None,
                    "registry cache is missing; using exact local host identity only".to_string(),
                    false,
                )],
            },
            Self::Spoke {
                cache: RoutineRegistryCacheView::Malformed { reason },
            } => RoutineRegistryStatus {
                source: "spoke_cache",
                state: "malformed",
                age_seconds: None,
                diagnostics: vec![diagnostic(
                    "registry_cache_malformed",
                    RoutineDiagnosticSeverity::Warning,
                    None,
                    format!(
                        "registry cache is malformed ({reason}); using exact local host identity only"
                    ),
                    false,
                )],
            },
            Self::Spoke {
                cache: RoutineRegistryCacheView::UnsupportedFuture { schema_version },
            } => RoutineRegistryStatus {
                source: "spoke_cache",
                state: "future_schema",
                age_seconds: None,
                diagnostics: vec![diagnostic(
                    "registry_cache_future_schema",
                    RoutineDiagnosticSeverity::Warning,
                    None,
                    format!(
                        "registry cache schema_version {schema_version} is newer than this Orbit; using exact local host identity only"
                    ),
                    false,
                )],
            },
        }
    }
}

/// Registry-neutral routine placement input. Higher-level composition can
/// obtain it from any catalog/cache implementation and pass it to Core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutinePlacementProjection {
    pub local_host: RoutineHostIdentity,
    pub registry: RoutineRegistryView,
}

/// Provider boundary for routine placement. Core's compatibility adapter is
/// registry-backed; an extracted remote feature can implement the same input
/// contract without making scheduling logic depend on that crate.
pub trait RoutinePlacementProvider {
    fn load_routine_placement(
        &self,
        now: DateTime<Utc>,
        cache_max_age: Duration,
    ) -> Result<RoutinePlacementProjection, OrbitError>;
}

/// Compatibility provider for the current on-disk host registry.
pub struct RegistryRoutinePlacementProvider<'a> {
    global_root: &'a Path,
    store: &'a Store,
    identity: &'a HostIdentity,
}

impl<'a> RegistryRoutinePlacementProvider<'a> {
    pub fn new(global_root: &'a Path, store: &'a Store, identity: &'a HostIdentity) -> Self {
        Self {
            global_root,
            store,
            identity,
        }
    }
}

impl RoutinePlacementProvider for RegistryRoutinePlacementProvider<'_> {
    fn load_routine_placement(
        &self,
        now: DateTime<Utc>,
        cache_max_age: Duration,
    ) -> Result<RoutinePlacementProjection, OrbitError> {
        let registry = load_routine_registry_view(
            self.global_root,
            self.store,
            self.identity,
            now,
            cache_max_age,
        )?;
        Ok(RoutinePlacementProjection {
            local_host: RoutineHostIdentity::from(self.identity),
            registry,
        })
    }
}

/// Load the appropriate local registry source. The spoke branch delegates all
/// byte decoding and freshness classification to `RegistryCacheService::load`.
pub fn load_routine_registry_view(
    global_root: &Path,
    store: &Store,
    identity: &HostIdentity,
    now: DateTime<Utc>,
    cache_max_age: Duration,
) -> Result<RoutineRegistryView, OrbitError> {
    match identity.mode {
        HostMode::Standalone => Ok(RoutineRegistryView::Standalone),
        HostMode::Hub => HostRegistryService::new(store.clone())
            .snapshot()
            .map(|snapshot| RoutineRegistryView::Hub { snapshot }),
        HostMode::Spoke => RegistryCacheService::new(global_root)
            .load(now, cache_max_age)
            .map(|cache| RoutineRegistryView::Spoke {
                cache: project_registry_cache(cache),
            }),
    }
}

fn project_registry_cache(cache: RegistryCacheState) -> RoutineRegistryCacheView {
    match cache {
        RegistryCacheState::Current { cache, age_seconds } => RoutineRegistryCacheView::Current {
            snapshot: Box::new(cache.snapshot),
            age_seconds,
        },
        RegistryCacheState::Stale { cache, age_seconds } => RoutineRegistryCacheView::Stale {
            snapshot: Box::new(cache.snapshot),
            age_seconds,
        },
        RegistryCacheState::Missing => RoutineRegistryCacheView::Missing,
        RegistryCacheState::Malformed { reason } => RoutineRegistryCacheView::Malformed { reason },
        RegistryCacheState::UnsupportedFuture { schema_version } => {
            RoutineRegistryCacheView::UnsupportedFuture { schema_version }
        }
    }
}

/// Pure, deterministic validation of one routine's declared pins.
pub fn validate_routine_pins(
    identity: &dyn RoutineHostIdentityView,
    origin: RoutineOrigin,
    pins: &[String],
    view: &RoutineRegistryView,
    now: DateTime<Utc>,
    quiet_after: Duration,
) -> RoutinePinValidation {
    if origin == RoutineOrigin::Local {
        return RoutinePinValidation {
            eligible: true,
            diagnostics: Vec::new(),
        };
    }

    let mut diagnostics = view.status().diagnostics;
    match view {
        RoutineRegistryView::Standalone => RoutinePinValidation {
            eligible: pins.iter().any(|pin| pin == identity.host_id()),
            diagnostics,
        },
        RoutineRegistryView::Hub { snapshot } => {
            let eligible = validate_snapshot_pins(
                identity,
                pins,
                snapshot,
                now,
                quiet_after,
                false,
                true,
                true,
                &mut diagnostics,
            );
            RoutinePinValidation {
                eligible,
                diagnostics,
            }
        }
        RoutineRegistryView::Spoke {
            cache: RoutineRegistryCacheView::Current { snapshot, .. },
        } => {
            let eligible = validate_snapshot_pins(
                identity,
                pins,
                snapshot,
                now,
                quiet_after,
                false,
                true,
                false,
                &mut diagnostics,
            );
            RoutinePinValidation {
                eligible,
                diagnostics,
            }
        }
        RoutineRegistryView::Spoke {
            cache: RoutineRegistryCacheView::Stale { snapshot, .. },
        } => {
            let eligible = validate_snapshot_pins(
                identity,
                pins,
                snapshot,
                now,
                quiet_after,
                true,
                false,
                false,
                &mut diagnostics,
            ) || pins.iter().any(|pin| pin == identity.host_id());
            RoutinePinValidation {
                eligible,
                diagnostics,
            }
        }
        RoutineRegistryView::Spoke { .. } => RoutinePinValidation {
            eligible: pins.iter().any(|pin| pin == identity.host_id()),
            diagnostics,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_snapshot_pins(
    identity: &dyn RoutineHostIdentityView,
    pins: &[String],
    snapshot: &RegistrySnapshotV1,
    now: DateTime<Utc>,
    quiet_after: Duration,
    stale: bool,
    authoritative: bool,
    allow_unregistered_local_identity: bool,
    diagnostics: &mut Vec<RoutineValidationDiagnostic>,
) -> bool {
    let mut eligible = false;
    for pin in pins {
        match resolve_pin(snapshot, pin) {
            PinResolution::Unknown
                if allow_unregistered_local_identity
                    && pin == identity.host_id()
                    && !snapshot
                        .hosts
                        .iter()
                        .any(|host| host.machine_id == identity.machine_id()) =>
            {
                // Upgrade compatibility: host.toml predates the registry and
                // remains trusted machine-local identity. Keep an exact local
                // committed pin firing until the hub is explicitly registered,
                // but make the missing registry record visible.
                eligible = true;
                diagnostics.push(diagnostic(
                    "local_host_unregistered",
                    RoutineDiagnosticSeverity::Warning,
                    Some(pin.clone()),
                    format!(
                        "exact local host pin '{pin}' is eligible from host.toml, but machine_id '{}' is not registered; run `orbit host register`",
                        identity.machine_id()
                    ),
                    false,
                ));
            }
            PinResolution::Unknown => diagnostics.push(diagnostic(
                "host_unknown",
                negative_severity(authoritative),
                Some(pin.clone()),
                if stale {
                    format!("host pin '{pin}' is unknown in stale registry data; observation is warning-only")
                } else {
                    format!("host pin '{pin}' is unknown in the current registry")
                },
                stale,
            )),
            PinResolution::Collision { machine_ids } => diagnostics.push(diagnostic(
                "host_collision",
                negative_severity(authoritative),
                Some(pin.clone()),
                format!(
                    "host pin '{pin}' is ambiguous across machine_ids [{}]{}",
                    machine_ids.join(", "),
                    stale_suffix(stale)
                ),
                stale,
            )),
            PinResolution::Retired { host } => diagnostics.push(diagnostic(
                "host_retired",
                negative_severity(authoritative),
                Some(pin.clone()),
                format!(
                    "host pin '{pin}' resolves to retired machine_id '{}'{}",
                    host.machine_id,
                    stale_suffix(stale)
                ),
                stale,
            )),
            PinResolution::Active { host, alias } => {
                if host.machine_id == identity.machine_id() {
                    eligible = true;
                }
                if alias {
                    diagnostics.push(diagnostic(
                        "host_alias",
                        RoutineDiagnosticSeverity::Warning,
                        Some(pin.clone()),
                        format!(
                            "host pin '{pin}' is a tombstone alias for '{}' ({}){}",
                            host.host_id,
                            host.machine_id,
                            stale_suffix(stale)
                        ),
                        stale,
                    ));
                }
                if let Some(last_seen_at) = host.last_seen_at {
                    let age = now.signed_duration_since(last_seen_at).num_seconds().max(0);
                    if age > quiet_after.num_seconds() {
                        diagnostics.push(diagnostic(
                            "host_quiet",
                            RoutineDiagnosticSeverity::Warning,
                            Some(pin.clone()),
                            format!(
                                "host '{}' ({}) is quiet (observed age {age}s){}",
                                host.host_id,
                                host.machine_id,
                                stale_suffix(stale)
                            ),
                            stale,
                        ));
                    }
                }
            }
        }
    }
    eligible
}

enum PinResolution<'a> {
    Active {
        host: &'a RegistryHostV1,
        alias: bool,
    },
    Retired {
        host: &'a RegistryHostV1,
    },
    Unknown,
    Collision {
        machine_ids: Vec<String>,
    },
}

fn resolve_pin<'a>(snapshot: &'a RegistrySnapshotV1, pin: &str) -> PinResolution<'a> {
    let mut matches: BTreeMap<&str, (&RegistryHostV1, bool)> = BTreeMap::new();
    for host in &snapshot.hosts {
        if host.host_id == pin {
            matches.insert(host.machine_id.as_str(), (host, false));
        }
        if host.aliases.iter().any(|alias| alias.alias_host_id == pin) {
            matches
                .entry(host.machine_id.as_str())
                .or_insert((host, true));
        }
    }
    if matches.len() > 1 {
        return PinResolution::Collision {
            machine_ids: matches.keys().map(|value| (*value).to_string()).collect(),
        };
    }
    let Some((host, alias)) = matches.into_values().next() else {
        return PinResolution::Unknown;
    };
    match host.status {
        HostStatus::Active => PinResolution::Active { host, alias },
        HostStatus::Retired => PinResolution::Retired { host },
    }
}

fn negative_severity(authoritative: bool) -> RoutineDiagnosticSeverity {
    if authoritative {
        RoutineDiagnosticSeverity::Error
    } else {
        RoutineDiagnosticSeverity::Warning
    }
}

fn stale_suffix(stale: bool) -> &'static str {
    if stale {
        "; observation came from stale registry data"
    } else {
        ""
    }
}

fn diagnostic(
    code: &'static str,
    severity: RoutineDiagnosticSeverity,
    pin: Option<String>,
    message: String,
    stale: bool,
) -> RoutineValidationDiagnostic {
    RoutineValidationDiagnostic {
        code,
        severity,
        pin,
        message,
        stale,
    }
}

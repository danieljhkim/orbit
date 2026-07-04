//! Routine definition schema (v1) — the durable, git-versioned unit of
//! scheduled work. One YAML file under `.orbit/routines/` in a routine-source
//! workspace describes a cron trigger, a catalog target, host pinning, and a
//! retry/overlap policy. See `docs/design/routines/2_design.md` [ORB-10021].
//!
//! Parsing is fail-closed: an invalid file is an error, never a routine that
//! fires with defaults. Targets are catalog references only — there is no
//! inline command form (ADR-0194 / ADR-0206 posture).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::activity_job::SchemaHeader;
use super::error::OrbitError;

/// Routine YAML schema version this binary reads and writes.
pub const ROUTINE_SCHEMA_VERSION: u32 = 1;

/// Prefix for job catalog target references (`job:<name>`).
pub const ROUTINE_JOB_TARGET_PREFIX: &str = "job:";
/// Prefix reserved for activity references. Not dispatchable in v1: run
/// dispatch is job-shaped (`submit_pipeline_run` resolves jobs by name),
/// so activities are fired by wrapping them in a one-step job in the same
/// source workspace.
pub const ROUTINE_ACTIVITY_TARGET_PREFIX: &str = "activity:";

/// A parsed, validated routine definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineDefinition {
    /// Schema version marker (`schemaVersion: 1`).
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// Unique routine name across all routine sources on a host.
    pub name: String,
    /// Human description of what the routine does.
    #[serde(default)]
    pub description: String,
    /// Versioned global kill-switch. Absent means enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Explicit host pinning — the routine fires only on hosts whose
    /// `host_id` appears here. There is no "any host" value in v1.
    pub hosts: Vec<String>,
    /// When the routine is due.
    pub trigger: RoutineTrigger,
    /// What fires: a catalog reference (`job:<name>`).
    pub target: RoutineTarget,
    /// Timeout, retry, and overlap handling applied by the dispatcher.
    #[serde(default)]
    pub policy: RoutinePolicy,
}

/// Cron trigger plus the policy for fires missed while the host was asleep
/// or powered off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineTrigger {
    /// Standard 5-field cron expression, evaluated in host-local time.
    pub cron: String,
    /// What to do about scheduled slots that fell in a gap. Defaults to
    /// `skip` (wait for the next natural slot).
    #[serde(default)]
    pub missed_run: MissedRunPolicy,
}

/// Missed-fire policy for gaps (sleep, downtime) between sweeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    /// Fire a single make-up run on the next sweep, no matter how many
    /// slots were missed.
    CatchUpOnce,
    /// Ignore missed slots; wait for the next natural one.
    #[default]
    Skip,
}

/// What a routine fires: a reference into the existing catalog, resolved at
/// load time. v1 dispatches `job:<name>` only — see
/// [`ROUTINE_ACTIVITY_TARGET_PREFIX`] for why `activity:` is reserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineTarget {
    /// A v2 job resolved by name through the job catalog.
    Job(String),
}

impl RoutineTarget {
    /// The catalog job name this target dispatches.
    pub fn job_name(&self) -> &str {
        match self {
            Self::Job(name) => name,
        }
    }

    /// Canonical string form (`job:<name>`).
    pub fn as_ref_string(&self) -> String {
        match self {
            Self::Job(name) => format!("{ROUTINE_JOB_TARGET_PREFIX}{name}"),
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        let value = raw.trim();
        if let Some(name) = value.strip_prefix(ROUTINE_JOB_TARGET_PREFIX) {
            let name = name.trim();
            if name.is_empty() {
                return Err("target 'job:' is missing a job name".to_string());
            }
            return Ok(Self::Job(name.to_string()));
        }
        if let Some(name) = value.strip_prefix(ROUTINE_ACTIVITY_TARGET_PREFIX) {
            return Err(format!(
                "target 'activity:{}' is not dispatchable in routines v1; wrap the \
                 activity in a one-step job in the same workspace and reference it \
                 as 'job:<name>'",
                name.trim()
            ));
        }
        Err(format!(
            "target '{value}' must be a catalog reference of the form 'job:<name>'; \
             inline commands are not supported (ADR-0194)"
        ))
    }
}

impl Serialize for RoutineTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_ref_string())
    }
}

impl<'de> Deserialize<'de> for RoutineTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Dispatcher policy applied around each fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutinePolicy {
    /// Ceiling on how long one fire may stay in flight before it stops
    /// blocking `overlap: forbid` (the staleness horizon) and is recorded
    /// as timed out.
    #[serde(default = "default_timeout_minutes")]
    pub timeout_minutes: u64,
    /// Bounded retries with fixed backoff for failed fires.
    #[serde(default)]
    pub retries: RoutineRetries,
    /// Whether a due fire may dispatch while a previous fire of the same
    /// routine is still in flight.
    #[serde(default)]
    pub overlap: OverlapPolicy,
}

impl Default for RoutinePolicy {
    fn default() -> Self {
        Self {
            timeout_minutes: default_timeout_minutes(),
            retries: RoutineRetries::default(),
            overlap: OverlapPolicy::default(),
        }
    }
}

/// Retry bounds for failed fires: up to `max` re-dispatches, each waiting at
/// least `backoff_minutes` after the previous failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineRetries {
    /// Maximum re-dispatches after the initial fire (0 = no retries).
    #[serde(default)]
    pub max: u32,
    /// Fixed delay between a failure and its retry.
    #[serde(default = "default_backoff_minutes")]
    pub backoff_minutes: u64,
}

impl Default for RoutineRetries {
    fn default() -> Self {
        Self {
            max: 0,
            backoff_minutes: default_backoff_minutes(),
        }
    }
}

/// Overlap handling when a fire comes due while another is in flight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    /// Skip the fire while one is in flight (the default).
    #[default]
    Forbid,
    /// Dispatch anyway.
    Allow,
}

const fn default_true() -> bool {
    true
}

const fn default_timeout_minutes() -> u64 {
    60
}

const fn default_backoff_minutes() -> u64 {
    2
}

/// Parse one routine YAML document fail-closed: schema-version gate first,
/// then a strict typed parse (unknown fields rejected), then semantic
/// validation. Any error means the routine is treated as absent by the
/// caller — it never degrades into "fire with defaults".
pub fn parse_routine_yaml(yaml: &str) -> Result<RoutineDefinition, OrbitError> {
    let header = SchemaHeader::parse_yaml(yaml)
        .map_err(|error| OrbitError::InvalidInput(format!("routine header: {error}")))?;
    if header.schema_version != ROUTINE_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported routine schemaVersion {} (this binary supports {})",
            header.schema_version, ROUTINE_SCHEMA_VERSION
        )));
    }
    let definition: RoutineDefinition = serde_yaml::from_str(yaml)
        .map_err(|error| OrbitError::InvalidInput(format!("routine: {error}")))?;
    definition.validate()?;
    Ok(definition)
}

impl RoutineDefinition {
    /// Semantic checks beyond serde shape: name charset, non-empty host
    /// pinning, non-empty cron. Full cron parsing happens in the scheduler
    /// (orbit-core), which owns the cron dependency.
    pub fn validate(&self) -> Result<(), OrbitError> {
        if !is_valid_routine_name(&self.name) {
            return Err(OrbitError::InvalidInput(format!(
                "routine name '{}' must be non-empty, lowercase alphanumeric \
                 with '-' or '_' separators, and start alphanumeric",
                self.name
            )));
        }
        if self.hosts.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "routine '{}' must pin at least one host (there is no \"any host\" in v1)",
                self.name
            )));
        }
        if self.hosts.iter().any(|host| host.trim().is_empty()) {
            return Err(OrbitError::InvalidInput(format!(
                "routine '{}' hosts must not contain empty entries",
                self.name
            )));
        }
        if self.trigger.cron.trim().is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "routine '{}' trigger.cron must not be empty",
                self.name
            )));
        }
        if self.policy.timeout_minutes == 0 {
            return Err(OrbitError::InvalidInput(format!(
                "routine '{}' policy.timeout_minutes must be at least 1",
                self.name
            )));
        }
        Ok(())
    }
}

fn is_valid_routine_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

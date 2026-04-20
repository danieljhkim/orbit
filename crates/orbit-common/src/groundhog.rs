//! Groundhog chronicle data structures and canonical serialization helpers.
//!
//! The append-only serializer contract follows `docs/design/groundhogv1.md` §5.3:
//! every later chronicle serialization must extend prior bytes instead of
//! rewriting them so prompt-cache breakpoints remain valid.
//!
//! Standard task-artifact persistence should serialize [`Chronicle`] itself via
//! serde. [`Chronicle::serialize_at`] and [`Chronicle::deserialize_cache_bytes`]
//! are cache-stream helpers for the append-only prefix format only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::types::{OrbitError, OrbitId};

/// Plan-authored checkpoint identifier (for example, `ckpt_01`).
pub type CheckpointId = String;

/// UTC timestamp used by Groundhog lineage records.
pub type Timestamp = DateTime<Utc>;

/// Long-lived Groundhog lineage for one task plan.
///
/// Persisted via `task.update` into the `artifacts.chronicle` task-artifact
/// field. The mutable [`Chronicle::deviation_stack`] is intentionally excluded
/// from [`Chronicle::serialize_at`] because v1 prompt-facing memory is derived
/// separately from the append-only chronicle body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chronicle {
    pub task_id: OrbitId,
    pub plan_id: OrbitId,
    #[serde(default)]
    pub days: Vec<Day>,
    #[serde(default)]
    pub deviation_stack: Vec<CheckpointId>,
}

impl Chronicle {
    const ARTIFACT_FIELD: &str = "artifacts.chronicle";
    const SCHEMA_VERSION: u32 = 1;

    /// Construct an empty Groundhog chronicle for one task plan.
    pub fn new(task_id: OrbitId, plan_id: OrbitId) -> Self {
        Self {
            task_id,
            plan_id,
            days: Vec::new(),
            deviation_stack: Vec::new(),
        }
    }

    /// Serialize this chronicle through `day_idx` as canonical append-only
    /// bytes. Each day is emitted as one JSON record per line so every earlier
    /// serialization is a byte-exact prefix of every later one.
    ///
    /// Returns an error when `day_idx` is outside the available day range.
    pub fn serialize_at(&self, day_idx: usize) -> Result<Vec<u8>, OrbitError> {
        if day_idx >= self.days.len() {
            return Err(OrbitError::InvalidInput(format!(
                "day_idx {day_idx} is out of bounds for chronicle with {} day(s)",
                self.days.len()
            )));
        }

        let day_count = day_idx + 1;
        let header = ChronicleHeader::from(self);

        let mut bytes =
            serde_json::to_vec(&header).expect("Groundhog header serialization is infallible");
        bytes.push(b'\n');

        for (day_index, day) in self.days.iter().take(day_count).enumerate() {
            let record = DayRecord::from_day(day_index, day);
            bytes.extend(
                serde_json::to_vec(&record)
                    .expect("Groundhog day-record serialization is infallible"),
            );
            bytes.push(b'\n');
        }

        Ok(bytes)
    }

    /// Deserialize the append-only cache stream emitted by
    /// [`Chronicle::serialize_at`].
    ///
    /// This only reconstructs the serialized chronicle prefix. The mutable
    /// `deviation_stack` is restored as empty because the design persists it
    /// separately from the append-only chronicle body.
    pub fn deserialize_cache_bytes(bytes: &[u8]) -> Result<Self, OrbitError> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "groundhog cache bytes must be valid UTF-8 JSON lines: {error}"
            ))
        })?;

        let mut lines = text.lines();
        let header_line = lines.next().ok_or_else(|| {
            OrbitError::InvalidInput("groundhog cache bytes must include a header line".to_string())
        })?;
        let header: ChronicleHeader = serde_json::from_str(header_line).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid groundhog cache header: {error}"))
        })?;
        header.validate()?;

        let mut days = Vec::new();
        for line in lines {
            let record: DayRecord = serde_json::from_str(line).map_err(|error| {
                OrbitError::InvalidInput(format!("invalid groundhog day record: {error}"))
            })?;
            if record.day_index != days.len() {
                return Err(OrbitError::InvalidInput(format!(
                    "groundhog day record index {} is out of sequence; expected {}",
                    record.day_index,
                    days.len()
                )));
            }
            days.push(record.into_day());
        }

        Ok(Self {
            task_id: header.task_id,
            plan_id: header.plan_id,
            days,
            deviation_stack: Vec::new(),
        })
    }
}

/// One checkpoint attempt lineage entry in the chronicle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Day {
    pub checkpoint_id: CheckpointId,
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    pub outcome: DayOutcome,
    pub summary: String,
    #[serde(default)]
    pub side_effects: Vec<SideEffect>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
}

/// Terminal outcome for one Groundhog day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DayOutcome {
    Success,
    Abandoned { reason: String },
    DeviatedTo(CheckpointId),
}

/// One executor attempt inside a day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attempt {
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_report: Option<FailureReport>,
    pub workspace_reverted: bool,
}

/// Distilled failure context that survives a retry boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureReport {
    pub what_tried: String,
    pub what_happened: String,
    pub next_attempt_plan: String,
}

/// Review-oriented tool-call summary retained for a single attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub seq: u32,
    pub tool_name: String,
    pub result_bytes: u64,
}

/// Persisted side effect that survived a checkpoint boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SideEffect {
    pub kind: SideEffectKind,
    pub target: String,
    pub reversible: bool,
}

/// Coarse side-effect classification for Groundhog summaries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectKind {
    FileWrite,
    FileDelete,
    GitCommit,
    DbMutation,
    Other,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChronicleHeader {
    schema_version: u32,
    artifact: String,
    task_id: String,
    plan_id: String,
}

impl From<&Chronicle> for ChronicleHeader {
    fn from(value: &Chronicle) -> Self {
        Self {
            schema_version: Chronicle::SCHEMA_VERSION,
            artifact: Chronicle::ARTIFACT_FIELD.to_string(),
            task_id: normalize_nfc(&value.task_id),
            plan_id: normalize_nfc(&value.plan_id),
        }
    }
}

impl ChronicleHeader {
    fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != Chronicle::SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported groundhog cache schema version {}; expected {}",
                self.schema_version,
                Chronicle::SCHEMA_VERSION
            )));
        }
        if self.artifact != Chronicle::ARTIFACT_FIELD {
            return Err(OrbitError::InvalidInput(format!(
                "groundhog cache header artifact '{}' does not match '{}'",
                self.artifact,
                Chronicle::ARTIFACT_FIELD
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DayRecord {
    day_index: usize,
    checkpoint_id: String,
    attempts: Vec<AttemptRecord>,
    outcome: NormalizedDayOutcome,
    summary: String,
    side_effects: Vec<NormalizedSideEffect>,
    started_at: Timestamp,
    ended_at: Timestamp,
}

impl DayRecord {
    fn from_day(day_index: usize, value: &Day) -> Self {
        Self {
            day_index,
            checkpoint_id: normalize_nfc(&value.checkpoint_id),
            attempts: value.attempts.iter().map(AttemptRecord::from).collect(),
            outcome: NormalizedDayOutcome::from(&value.outcome),
            summary: normalize_nfc(&value.summary),
            side_effects: value
                .side_effects
                .iter()
                .map(NormalizedSideEffect::from)
                .collect(),
            started_at: value.started_at.to_owned(),
            ended_at: value.ended_at.to_owned(),
        }
    }

    fn into_day(self) -> Day {
        Day {
            checkpoint_id: self.checkpoint_id,
            attempts: self
                .attempts
                .into_iter()
                .map(AttemptRecord::into_attempt)
                .collect(),
            outcome: self.outcome.into_day_outcome(),
            summary: self.summary,
            side_effects: self
                .side_effects
                .into_iter()
                .map(NormalizedSideEffect::into_side_effect)
                .collect(),
            started_at: self.started_at,
            ended_at: self.ended_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AttemptRecord {
    started_at: Timestamp,
    ended_at: Timestamp,
    tool_calls: Vec<NormalizedToolCallRecord>,
    failure_report: Option<NormalizedFailureReport>,
    workspace_reverted: bool,
}

impl From<&Attempt> for AttemptRecord {
    fn from(value: &Attempt) -> Self {
        Self {
            started_at: value.started_at.to_owned(),
            ended_at: value.ended_at.to_owned(),
            tool_calls: value
                .tool_calls
                .iter()
                .map(NormalizedToolCallRecord::from)
                .collect(),
            failure_report: value
                .failure_report
                .as_ref()
                .map(NormalizedFailureReport::from),
            workspace_reverted: value.workspace_reverted,
        }
    }
}

impl AttemptRecord {
    fn into_attempt(self) -> Attempt {
        Attempt {
            started_at: self.started_at,
            ended_at: self.ended_at,
            tool_calls: self
                .tool_calls
                .into_iter()
                .map(NormalizedToolCallRecord::into_tool_call_record)
                .collect(),
            failure_report: self
                .failure_report
                .map(NormalizedFailureReport::into_failure_report),
            workspace_reverted: self.workspace_reverted,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NormalizedDayOutcome {
    Success,
    Abandoned { reason: String },
    DeviatedTo(String),
}

impl From<&DayOutcome> for NormalizedDayOutcome {
    fn from(value: &DayOutcome) -> Self {
        match value {
            DayOutcome::Success => Self::Success,
            DayOutcome::Abandoned { reason } => Self::Abandoned {
                reason: normalize_nfc(reason),
            },
            DayOutcome::DeviatedTo(checkpoint_id) => Self::DeviatedTo(normalize_nfc(checkpoint_id)),
        }
    }
}

impl NormalizedDayOutcome {
    fn into_day_outcome(self) -> DayOutcome {
        match self {
            Self::Success => DayOutcome::Success,
            Self::Abandoned { reason } => DayOutcome::Abandoned { reason },
            Self::DeviatedTo(checkpoint_id) => DayOutcome::DeviatedTo(checkpoint_id),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct NormalizedFailureReport {
    what_tried: String,
    what_happened: String,
    next_attempt_plan: String,
}

impl From<&FailureReport> for NormalizedFailureReport {
    fn from(value: &FailureReport) -> Self {
        Self {
            what_tried: normalize_nfc(&value.what_tried),
            what_happened: normalize_nfc(&value.what_happened),
            next_attempt_plan: normalize_nfc(&value.next_attempt_plan),
        }
    }
}

impl NormalizedFailureReport {
    fn into_failure_report(self) -> FailureReport {
        FailureReport {
            what_tried: self.what_tried,
            what_happened: self.what_happened,
            next_attempt_plan: self.next_attempt_plan,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct NormalizedToolCallRecord {
    seq: u32,
    tool_name: String,
    result_bytes: u64,
}

impl From<&ToolCallRecord> for NormalizedToolCallRecord {
    fn from(value: &ToolCallRecord) -> Self {
        Self {
            seq: value.seq,
            tool_name: normalize_nfc(&value.tool_name),
            result_bytes: value.result_bytes,
        }
    }
}

impl NormalizedToolCallRecord {
    fn into_tool_call_record(self) -> ToolCallRecord {
        ToolCallRecord {
            seq: self.seq,
            tool_name: self.tool_name,
            result_bytes: self.result_bytes,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct NormalizedSideEffect {
    kind: SideEffectKind,
    target: String,
    reversible: bool,
}

impl From<&SideEffect> for NormalizedSideEffect {
    fn from(value: &SideEffect) -> Self {
        Self {
            kind: value.kind.clone(),
            target: normalize_nfc(&value.target),
            reversible: value.reversible,
        }
    }
}

impl NormalizedSideEffect {
    fn into_side_effect(self) -> SideEffect {
        SideEffect {
            kind: self.kind,
            target: self.target,
            reversible: self.reversible,
        }
    }
}

fn normalize_nfc(value: &str) -> String {
    value.nfc().collect()
}

//! Project learning types.
//!
//! A [`Learning`] is a durable, structured note that captures non-obvious
//! project knowledge — the kind of thing that would otherwise live as a
//! one-off comment in a single PR. Learnings are workspace-scoped, checked
//! into git, and surfaced via push injection — engine pre-prompt
//! (`orbit-engine`'s `maybe_prepend_learning_reminders`) and an MCP sidecar
//! decorator (`orbit-remote`'s `LearningSidecarDecorator`) — plus pull
//! retrieval via `orbit search` / `orbit learning show`. The Claude Code
//! `PreToolUse` hook layer was retired [ORB-10346]; the other two push
//! layers remain active on every run.
//!
//! Phase 1's on-disk schema reserves `scope.symbols` and
//! `scope.semantic_seed` for phase-2 symbol-aware scope and semantic
//! ranking. Both fields deserialize via `#[serde(default)]` and round-trip
//! unchanged so a phase-1 store can read forward-compatible fixtures
//! without loss.

use std::collections::BTreeSet;
use std::env;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::OrbitId;

/// Lifecycle state of a learning record.
///
/// Phase 1 has only two states; `Superseded` is reached via the explicit
/// [`Learning::superseded_by`] / [`Learning::supersedes`] link, never via a
/// bare status flip.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningStatus {
    Active,
    Superseded,
}

impl Display for LearningStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl LearningStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LearningStatus::Active => "active",
            LearningStatus::Superseded => "superseded",
        }
    }
}

impl FromStr for LearningStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(LearningStatus::Active),
            "superseded" => Ok(LearningStatus::Superseded),
            other => Err(format!("unknown learning status: {other}")),
        }
    }
}

/// Kind of evidence attached to a learning.
///
/// The variant determines how `reference` is interpreted:
/// - `Task` — an Orbit task ID (e.g. `T20260510-7`).
/// - `Commit` — a git revision (short or long SHA).
/// - `External` — an opaque pointer (URL, ticket, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Task,
    Commit,
    External,
}

impl Display for EvidenceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EvidenceKind::Task => "task",
            EvidenceKind::Commit => "commit",
            EvidenceKind::External => "external",
        };
        f.write_str(s)
    }
}

impl FromStr for EvidenceKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "task" => Ok(EvidenceKind::Task),
            "commit" => Ok(EvidenceKind::Commit),
            "external" => Ok(EvidenceKind::External),
            other => Err(format!("unknown evidence kind: {other}")),
        }
    }
}

/// A single piece of evidence supporting a learning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningEvidence {
    pub kind: EvidenceKind,
    pub reference: String,
}

/// Scope under which a learning applies.
///
/// Phase 1 evaluates `paths` (glob match) OR `tags` (exact match). The
/// remaining two fields are reserved for future use and persist verbatim:
/// - `symbols` — symbol-aware scope identifiers for a future resolver.
/// - `semantic_seed` — a representative passage used to compute embedding
///   similarity at query time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LearningScope {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Reserved for phase-2 symbol-aware scope. Not read in phase 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    /// Reserved for phase-2 semantic ranking. Not read in phase 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_seed: Option<String>,
}

/// A persisted project learning record.
///
/// The on-disk YAML shape closely mirrors this struct via the
/// `LearningFileDocument` wrapper in `orbit-store`. Field naming follows
/// the same conventions as [`crate::types::Task`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Learning {
    pub id: OrbitId,
    pub status: LearningStatus,
    pub scope: LearningScope,
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub evidence: Vec<LearningEvidence>,
    /// ID of the learning this record replaces, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<OrbitId>,
    /// ID of the learning that supersedes this record, if any. Mutually
    /// exclusive with `status = Active` for well-formed records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<OrbitId>,
    /// Historical IDs retained after format migrations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_ids: Vec<OrbitId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Optional priority used as a secondary key in `search` ranking.
    /// Higher values rank first; `None` sorts after all `Some(_)` values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

pub const DEFAULT_LEARNING_REMINDER_PER_CALL_CAP: usize = 5;
pub const DEFAULT_LEARNING_REMINDER_SESSION_CAP: usize = 20;

/// Envelope projected into agent context by the project-learnings injection
/// layers. The teaser carries only the id, one-line summary, and scope tags;
/// agents open the full body via `orbit.learning.show` when they need the
/// details — that show is the passive usage signal ([ORB-10316]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningReminder {
    pub id: OrbitId,
    pub summary: String,
    /// Scope tags for the learning, rendered inline in the teaser. Empty when
    /// the learning is path-scoped only; `#[serde(default)]` keeps older
    /// persisted dedup state readable.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Budget controls for project-learning injection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningInjectionCaps {
    pub per_call: usize,
    pub per_session_hard: usize,
}

impl Default for LearningInjectionCaps {
    fn default() -> Self {
        Self {
            per_call: DEFAULT_LEARNING_REMINDER_PER_CALL_CAP,
            per_session_hard: DEFAULT_LEARNING_REMINDER_SESSION_CAP,
        }
    }
}

impl LearningInjectionCaps {
    /// Read documented cap overrides from the environment.
    ///
    /// Invalid or zero values fall back to defaults so a bad shell export does
    /// not disable the learning-injection path.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            per_call: read_cap_env("ORBIT_LEARNING_PER_CALL_CAP").unwrap_or(defaults.per_call),
            per_session_hard: read_cap_env("ORBIT_LEARNING_SESSION_CAP")
                .unwrap_or(defaults.per_session_hard),
        }
    }
}

fn read_cap_env(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

/// Per-session deduplication state for all learning-injection layers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LearningInjectionState {
    #[serde(default)]
    pub emitted_ids: BTreeSet<OrbitId>,
    #[serde(default)]
    pub count: usize,
}

impl LearningInjectionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seeded(ids: impl IntoIterator<Item = OrbitId>) -> Self {
        let emitted_ids: BTreeSet<_> = ids.into_iter().collect();
        let count = emitted_ids.len();
        Self { emitted_ids, count }
    }

    /// Admit a learning ID if it is new and the hard cap has not been reached.
    ///
    /// Deduplication and the hard cap are intentionally separate gates:
    /// duplicates never consume cap, while new IDs stop once the hard cap is
    /// reached.
    pub fn try_admit(&mut self, id: &str, caps: LearningInjectionCaps) -> bool {
        if self.emitted_ids.contains(id) {
            return false;
        }
        if self.count >= caps.per_session_hard {
            return false;
        }
        self.emitted_ids.insert(id.to_string());
        self.count += 1;
        true
    }

    /// Return the reminders newly admitted for this call, honoring both the
    /// per-call cap and the per-session hard cap.
    pub fn admit_reminders(
        &mut self,
        reminders: &[LearningReminder],
        caps: LearningInjectionCaps,
    ) -> Vec<LearningReminder> {
        let mut admitted = Vec::with_capacity(caps.per_call.min(reminders.len()));
        for reminder in reminders {
            if admitted.len() >= caps.per_call {
                break;
            }
            if self.try_admit(&reminder.id, caps) {
                admitted.push(reminder.clone());
            }
        }
        admitted
    }
}

/// Render a project-learning reminder block in the prompt format documented in
/// `docs/design/project-learnings/2_design.md` §4.1.
pub fn render_reminder_block(reminders: &[LearningReminder]) -> String {
    if reminders.is_empty() {
        return String::new();
    }

    let mut out = String::from("<system-reminder>\n");
    out.push_str("Project learnings relevant to this task:\n\n");
    for reminder in reminders {
        if reminder.tags.is_empty() {
            out.push_str(&format!("- [{}] {}\n", reminder.id, reminder.summary));
        } else {
            out.push_str(&format!(
                "- [{}] {} [tags: {}]\n",
                reminder.id,
                reminder.summary,
                reminder.tags.join(", ")
            ));
        }
    }
    out.push('\n');
    out.push_str("Read full body via `orbit.learning.show <id>` if needed.\n");
    out.push_str("</system-reminder>");
    out
}

/// Prepend rendered reminders to an existing prompt, preserving byte-for-byte
/// identity when there are no reminders.
pub fn prepend_reminder_block(prompt: &str, reminders: &[LearningReminder]) -> String {
    let block = render_reminder_block(reminders);
    if block.is_empty() {
        return prompt.to_string();
    }
    if prompt.is_empty() {
        block
    } else {
        format!("{block}\n\n{prompt}")
    }
}

/// Lowercase + trim + dedupe a list of tag strings. Mirrors
/// [`crate::types::normalize_task_tags`].
pub fn normalize_learning_tags(raw_tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(raw_tags.len());
    let mut seen = BTreeSet::new();
    for raw in raw_tags {
        let tag = raw.trim().to_lowercase();
        if !tag.is_empty() && seen.insert(tag.clone()) {
            normalized.push(tag);
        }
    }
    normalized
}

/// Trim + dedupe a list of path-glob strings, preserving the first occurrence
/// of each unique pattern. Paths are not lowercased — globs are case-sensitive.
pub fn normalize_learning_paths(raw_paths: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(raw_paths.len());
    let mut seen = BTreeSet::new();
    for raw in raw_paths {
        let path = raw.trim().to_string();
        if !path.is_empty() && seen.insert(path.clone()) {
            normalized.push(path);
        }
    }
    normalized
}

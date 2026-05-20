//! Project learning types.
//!
//! A [`Learning`] is a durable, structured note that captures non-obvious
//! project knowledge — the kind of thing that would otherwise live as a
//! one-off comment in a single PR. Learnings are workspace-scoped, checked
//! into git, and surfaced via the three-layer push-injection pipeline
//! (engine pre-prompt, MCP sidecar, Claude Code hook).
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
/// remaining two fields are reserved for phase 2 and persist verbatim:
/// - `symbols` — symbol-aware scope (`module::ident` IDs from the
///   knowledge graph).
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

/// Append-only vote event for an existing learning.
///
/// Vote rows are projection metadata stored beside the learning YAML record
/// in `votes.jsonl`; they are not part of the persisted `Learning` document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningVoteRow {
    pub learning_id: OrbitId,
    pub voter_model: String,
    pub voted_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<OrbitId>,
}

/// Derived vote statistics for a learning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LearningVoteSummary {
    pub vote_count: usize,
    pub last_voted_at: Option<DateTime<Utc>>,
}

/// Append-only comment anchored to exactly one learning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningComment {
    pub id: OrbitId,
    pub learning_id: OrbitId,
    pub body: String,
    pub author_model: String,
    pub created_at: DateTime<Utc>,
}

/// Tombstone event that soft-deletes a learning comment without rewriting the
/// original create row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningCommentTombstone {
    pub id: OrbitId,
    pub learning_id: OrbitId,
    #[serde(default = "learning_comment_delete_op")]
    pub op: String,
    pub deleted_at: DateTime<Utc>,
    pub deleted_by: String,
}

fn learning_comment_delete_op() -> String {
    "delete".to_string()
}

/// JSONL row stored in `.orbit/learnings/<L-id>/comments.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LearningCommentEvent {
    Create(LearningComment),
    Tombstone(LearningCommentTombstone),
}

/// Compute the decay-weighted score for vote timestamps at `now`.
///
/// A half-life of `0.0` disables decay and returns the raw vote count.
pub fn decayed_vote_score(
    voted_at_values: &[DateTime<Utc>],
    now: DateTime<Utc>,
    half_life_days: f64,
) -> f64 {
    if half_life_days == 0.0 {
        return voted_at_values.len() as f64;
    }

    voted_at_values
        .iter()
        .map(|voted_at| {
            let age_days =
                now.signed_duration_since(*voted_at).num_milliseconds() as f64 / 86_400_000.0;
            2_f64.powf(-age_days / half_life_days)
        })
        .sum()
}

pub const DEFAULT_LEARNING_REMINDER_PER_CALL_CAP: usize = 5;
pub const DEFAULT_LEARNING_REMINDER_SESSION_CAP: usize = 20;
pub const DEFAULT_LEARNING_COMMENT_RENDER_CAP: usize = 3;

/// Envelope projected into agent context by the project-learnings injection
/// layers. The learning itself carries only the summary; short anchored
/// comments are optional footnotes rendered beneath it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningReminder {
    pub id: OrbitId,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<LearningComment>,
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

pub fn read_comment_render_cap_env() -> usize {
    read_cap_env("ORBIT_LEARNING_COMMENT_RENDER_CAP").unwrap_or(DEFAULT_LEARNING_COMMENT_RENDER_CAP)
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
    let comment_cap = read_comment_render_cap_env();
    for reminder in reminders {
        out.push_str(&format!("- [{}] {}\n", reminder.id, reminder.summary));
        for comment in reminder.comments.iter().take(comment_cap) {
            let first_line = comment.body.lines().next().unwrap_or("").trim();
            out.push_str(&format!("  - [{}] {}\n", comment.id, first_line));
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_learning() -> Learning {
        let ts = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        Learning {
            id: "L-0001".to_string(),
            status: LearningStatus::Active,
            scope: LearningScope {
                paths: vec!["crates/orbit-engine/**/perf*.rs".to_string()],
                tags: vec!["performance".to_string()],
                symbols: vec!["orbit_engine::perf_runner::run".to_string()],
                semantic_seed: Some("benchmark equivalence check".to_string()),
            },
            summary: "Verify output equivalence on perf changes.".to_string(),
            body: "Full body here.".to_string(),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: "T20260510-1".to_string(),
            }],
            supersedes: None,
            superseded_by: None,
            legacy_ids: Vec::new(),
            created_at: ts,
            updated_at: ts,
            created_by: Some("claude-opus-4-7".to_string()),
            priority: None,
        }
    }

    #[test]
    fn normalize_learning_tags_trims_lowercases_and_dedupes() {
        let tags = normalize_learning_tags(vec![
            "  Perf ".to_string(),
            "BENCH".to_string(),
            "perf".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(tags, vec!["perf", "bench"]);
    }

    #[test]
    fn normalize_learning_paths_trims_and_dedupes_preserving_case() {
        let paths = normalize_learning_paths(vec![
            "  crates/Foo/**  ".to_string(),
            "crates/Foo/**".to_string(),
            "crates/Bar/*.rs".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(paths, vec!["crates/Foo/**", "crates/Bar/*.rs"]);
    }

    #[test]
    fn learning_yaml_round_trips_reserved_phase_two_fields() {
        let learning = sample_learning();
        let yaml = serde_yaml::to_string(&learning).expect("serialize");
        assert!(yaml.contains("symbols:"));
        assert!(yaml.contains("semantic_seed: benchmark equivalence check"));

        let decoded: Learning = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(decoded, learning);
    }

    #[test]
    fn learning_loads_minimal_yaml_with_phase_two_defaults() {
        let yaml = r#"id: L-0002
status: active
scope:
  paths: []
  tags: []
summary: Minimal record
body: ''
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
"#;
        let learning: Learning = serde_yaml::from_str(yaml).expect("deserialize");
        assert!(learning.scope.symbols.is_empty());
        assert!(learning.scope.semantic_seed.is_none());
        assert!(learning.evidence.is_empty());
        assert_eq!(learning.id, "L-0002");
        assert_eq!(learning.status, LearningStatus::Active);
    }

    #[test]
    fn learning_status_from_str_round_trips() {
        for status in [LearningStatus::Active, LearningStatus::Superseded] {
            let parsed: LearningStatus = status.as_str().parse().expect("parse");
            assert_eq!(parsed, status);
        }
        assert!(LearningStatus::from_str("nope").is_err());
    }

    #[test]
    fn render_reminder_block_returns_empty_for_no_reminders() {
        assert_eq!(render_reminder_block(&[]), "");
        assert_eq!(prepend_reminder_block("baseline", &[]), "baseline");
    }

    #[test]
    fn render_reminder_block_matches_design_shape() {
        let block = render_reminder_block(&[LearningReminder {
            id: "L-0001".to_string(),
            summary: "Verify output equivalence before freezing a result.".to_string(),
            comments: Vec::new(),
        }]);

        assert_eq!(
            block,
            "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [L-0001] Verify output equivalence before freezing a result.\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>"
        );
    }

    #[test]
    fn learning_injection_state_dedupes_without_consuming_hard_cap() {
        let caps = LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 2,
        };
        let mut state = LearningInjectionState::new();

        assert!(state.try_admit("L1", caps));
        assert!(!state.try_admit("L1", caps));
        assert!(state.try_admit("L2", caps));
        assert!(!state.try_admit("L3", caps));
        assert_eq!(state.count, 2);
        assert_eq!(state.emitted_ids.len(), 2);
    }

    #[test]
    fn admit_reminders_enforces_per_call_cap() {
        let caps = LearningInjectionCaps {
            per_call: 2,
            per_session_hard: 20,
        };
        let mut state = LearningInjectionState::new();
        let reminders: Vec<_> = (0..4)
            .map(|idx| LearningReminder {
                id: format!("L{idx}"),
                summary: format!("summary {idx}"),
                comments: Vec::new(),
            })
            .collect();

        let admitted = state.admit_reminders(&reminders, caps);

        assert_eq!(
            admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L0", "L1"]
        );
        assert_eq!(state.count, 2);
    }

    #[test]
    fn evidence_kind_from_str_covers_all_variants() {
        assert_eq!(
            EvidenceKind::from_str("task").expect("task"),
            EvidenceKind::Task
        );
        assert_eq!(
            EvidenceKind::from_str("commit").expect("commit"),
            EvidenceKind::Commit
        );
        assert_eq!(
            EvidenceKind::from_str("external").expect("external"),
            EvidenceKind::External
        );
        assert!(EvidenceKind::from_str("other").is_err());
    }

    #[test]
    fn decayed_vote_score_halves_each_half_life() {
        let now = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
        let recent = now;
        let old = now - chrono::Duration::days(180);

        let recent_weight = decayed_vote_score(&[recent], now, 180.0);
        let old_weight = decayed_vote_score(&[old], now, 180.0);

        let ratio = recent_weight / old_weight;
        assert!(
            (ratio - 2.0).abs() < 1e-6,
            "expected 2:1 ratio, got {ratio}"
        );
    }

    #[test]
    fn decayed_vote_score_zero_half_life_returns_raw_count() {
        let now = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
        let votes = [
            now - chrono::Duration::days(30),
            now - chrono::Duration::days(730),
            now - chrono::Duration::days(1460),
        ];

        assert_eq!(decayed_vote_score(&votes, now, 0.0), 3.0);
    }

    #[test]
    fn render_reminder_block_renders_comments_under_learning() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
        let block = render_reminder_block(&[LearningReminder {
            id: "L-0001".to_string(),
            summary: "Remember the important thing.".to_string(),
            comments: vec![LearningComment {
                id: "C20260517-1".to_string(),
                learning_id: "L-0001".to_string(),
                body: "Use the narrow helper.\nExtra detail stays hidden.".to_string(),
                author_model: "codex".to_string(),
                created_at: ts,
            }],
        }]);

        assert!(block.contains("- [L-0001] Remember the important thing.\n"));
        assert!(block.contains("  - [C20260517-1] Use the narrow helper.\n"));
    }

    #[test]
    fn forward_compat_fixture_with_symbols_and_semantic_seed_round_trips() {
        let yaml = r#"id: L-0003
status: active
scope:
  paths:
    - "crates/orbit-engine/**"
  tags:
    - performance
  symbols:
    - "a::b"
  semantic_seed: "x"
summary: Fixture with phase-2 fields
body: ''
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
"#;
        let learning: Learning = serde_yaml::from_str(yaml).expect("deserialize");
        assert_eq!(learning.scope.symbols, vec!["a::b"]);
        assert_eq!(learning.scope.semantic_seed.as_deref(), Some("x"));

        let yaml_out = serde_yaml::to_string(&learning).expect("serialize");
        let round_tripped: Learning = serde_yaml::from_str(&yaml_out).expect("deserialize 2");
        assert_eq!(round_tripped, learning);
    }
}

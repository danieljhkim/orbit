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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
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
            id: "L20260511-1".to_string(),
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
            created_at: ts,
            updated_at: ts,
            created_by: Some("claude-opus-4-7".to_string()),
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
        let yaml = r#"id: L20260511-2
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
        assert_eq!(learning.id, "L20260511-2");
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
    fn forward_compat_fixture_with_symbols_and_semantic_seed_round_trips() {
        let yaml = r#"id: L20260511-3
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

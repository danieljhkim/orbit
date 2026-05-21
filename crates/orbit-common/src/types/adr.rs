//! Architecture Decision Record (ADR) types: status lifecycle, legacy
//! validation hints, the [`Adr`] struct itself, and ID helpers.
//!
//! ## ADR Status Lifecycle
//!
//! ADRs move through a small, restrictive state machine — unlike tasks (which
//! are permissive by default), ADR transitions are an explicit allowlist.
//!
//! ### Allowed transitions
//! | From       | To         | Notes                                              |
//! |------------|------------|----------------------------------------------------|
//! | Proposed   | Accepted   | Standard promotion path.                           |
//! | Proposed   | Superseded | Withdrawn before acceptance.                       |
//! | Proposed   | Deleted    | Soft-discard a never-accepted decision.            |
//! | Accepted   | Superseded | Replaced by a newer decision.                      |
//! | `X`        | `X`        | Same-state transitions are idempotent no-ops.      |
//!
//! ### Rejected transitions
//! - `Accepted → Proposed` — once accepted, cannot revert to proposed.
//! - `Accepted → Deleted` — accepted decisions must be superseded, not deleted.
//! - `Superseded → *` — terminal.
//! - `Deleted → *` — terminal.
//!
//! See [`AdrStatus::validate_transition`] for the implementation.

use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::OrbitError;

/// Current lifecycle state of an ADR.
///
/// See the module-level doc for the full transition table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum AdrStatus {
    /// Drafted but not yet accepted by the feature owner.
    Proposed,
    /// Active, in-force decision.
    Accepted,
    /// Replaced by a newer ADR (see `superseded_by`). Terminal.
    Superseded,
    /// Soft-discarded before acceptance. Terminal.
    Deleted,
}

impl Display for AdrStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cli_name())
    }
}

impl FromStr for AdrStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "proposed" => Ok(AdrStatus::Proposed),
            "accepted" => Ok(AdrStatus::Accepted),
            "superseded" => Ok(AdrStatus::Superseded),
            "deleted" => Ok(AdrStatus::Deleted),
            other => Err(format!("unknown ADR status: {other}")),
        }
    }
}

impl AdrStatus {
    pub fn cli_name(self) -> &'static str {
        match self {
            AdrStatus::Proposed => "proposed",
            AdrStatus::Accepted => "accepted",
            AdrStatus::Superseded => "superseded",
            AdrStatus::Deleted => "deleted",
        }
    }

    /// Validates a status transition against the ADR allowlist.
    ///
    /// Same-state transitions are idempotent OK. Everything else must match an
    /// allowed edge; otherwise returns [`OrbitError::AdrInvalidTransition`].
    pub fn validate_transition(from: AdrStatus, to: AdrStatus) -> Result<(), OrbitError> {
        if from == to {
            return Ok(());
        }

        match (from, to) {
            (AdrStatus::Proposed, AdrStatus::Accepted)
            | (AdrStatus::Proposed, AdrStatus::Superseded)
            | (AdrStatus::Proposed, AdrStatus::Deleted)
            | (AdrStatus::Accepted, AdrStatus::Superseded) => Ok(()),
            (AdrStatus::Accepted, AdrStatus::Proposed) => Err(OrbitError::AdrInvalidTransition(
                format!("{from} -> {to} (accepted ADRs cannot revert to proposed)"),
            )),
            (AdrStatus::Accepted, AdrStatus::Deleted) => Err(OrbitError::AdrInvalidTransition(
                format!("{from} -> {to} (accepted ADRs must be superseded, not deleted)"),
            )),
            (AdrStatus::Superseded, _) => Err(OrbitError::AdrInvalidTransition(format!(
                "{from} -> {to} (superseded is terminal)"
            ))),
            (AdrStatus::Deleted, _) => Err(OrbitError::AdrInvalidTransition(format!(
                "{from} -> {to} (deleted is terminal)"
            ))),
            _ => Err(OrbitError::AdrInvalidTransition(format!(
                "{from} -> {to} (not an allowed transition)"
            ))),
        }
    }
}

/// Whether an ADR has been flagged with legacy-validation warnings.
///
/// See ADR-011 of the adr-artifact design. `Warned` means
/// [`Adr::validation_warnings`] is non-empty and the record was admitted
/// despite failing one or more soft checks.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum LegacyValidation {
    /// No legacy validation warnings; record passed all checks.
    #[default]
    None,
    /// One or more soft validation warnings recorded in `validation_warnings`.
    Warned,
}

impl Display for LegacyValidation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LegacyValidation::None => "none",
            LegacyValidation::Warned => "warned",
        };
        write!(f, "{s}")
    }
}

impl FromStr for LegacyValidation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(LegacyValidation::None),
            "warned" => Ok(LegacyValidation::Warned),
            other => Err(format!("unknown legacy validation: {other}")),
        }
    }
}

/// Architecture Decision Record.
///
/// See `docs/design/adr-artifact/2_design.md` §1 for the canonical schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Adr {
    pub id: String,
    pub title: String,
    pub status: AdrStatus,
    pub owner: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<DateTime<Utc>>,
    pub last_updated: DateTime<Utc>,
    #[serde(default)]
    pub related_features: Vec<String>,
    #[serde(default)]
    pub related_tasks: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Legacy ID aliases. Array per ADR-002 (amended) to support rollup
    /// aliasing where multiple legacy IDs collapse into a single canonical ADR.
    #[serde(default)]
    pub legacy_ids: Vec<String>,
    /// Soft-validation warnings recorded during ingestion. See ADR-011.
    #[serde(default)]
    pub validation_warnings: Vec<String>,
    /// Summary flag mirroring whether `validation_warnings` is populated.
    /// See ADR-011.
    #[serde(default)]
    pub legacy_validation: LegacyValidation,
}

/// Validates a canonical ADR ID of form `ADR-NNNN` (at least 4 zero-padded
/// digits).
///
/// Rejects empty strings, missing prefix, lowercase prefix, fewer than 4
/// digits, or any non-digit suffix character. Uses character checks rather
/// than the `regex` crate so this stays usable in runtime code paths.
pub fn validate_adr_id(id: &str) -> Result<(), OrbitError> {
    if id.is_empty() {
        return Err(OrbitError::InvalidInput(
            "ADR id must not be empty".to_string(),
        ));
    }

    let suffix = id.strip_prefix("ADR-").ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "ADR id '{id}' must start with 'ADR-' (uppercase prefix)"
        ))
    })?;

    if suffix.len() < 4 {
        return Err(OrbitError::InvalidInput(format!(
            "ADR id '{id}' must have at least 4 digits after 'ADR-'"
        )));
    }

    if !suffix.chars().all(|c| c.is_ascii_digit()) {
        return Err(OrbitError::InvalidInput(format!(
            "ADR id '{id}' suffix must contain only ASCII digits"
        )));
    }

    Ok(())
}

/// Formats a legacy, feature-scoped ADR ID used in the markdown design docs,
/// e.g. `legacy_id_for("activity-job", 17) == "activity-job/ADR-017"`.
///
/// The local number is zero-padded to 3 digits to match the existing
/// `<feature>/ADR-NNN` markdown convention. Wider numbers are emitted at their
/// natural width.
pub fn legacy_id_for(feature: &str, local_number: u32) -> String {
    format!("{feature}/ADR-{local_number:03}")
}

/// Lowercase + trim + dedupe ADR tag strings.
///
/// ADR tags share the task/learning free-form label semantics, while
/// `related_features` remains the constrained structural feature reference.
pub fn normalize_adr_tags(raw_tags: Vec<String>) -> Vec<String> {
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

/// Trim + dedupe ADR applicability path globs, preserving case.
pub fn normalize_adr_paths(raw_paths: Vec<String>) -> Vec<String> {
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
    use super::*;
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    // --- AdrStatus::validate_transition happy paths --------------------------

    #[test]
    fn transition_proposed_to_accepted_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Accepted)
            .expect("proposed -> accepted should be allowed");
    }

    #[test]
    fn transition_proposed_to_superseded_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Superseded)
            .expect("proposed -> superseded should be allowed");
    }

    #[test]
    fn transition_proposed_to_deleted_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Deleted)
            .expect("proposed -> deleted should be allowed");
    }

    #[test]
    fn transition_accepted_to_superseded_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Superseded)
            .expect("accepted -> superseded should be allowed");
    }

    #[test]
    fn transition_same_state_is_idempotent_for_all_variants() {
        for status in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
            AdrStatus::Deleted,
        ] {
            AdrStatus::validate_transition(status, status)
                .expect("same-state transition should be idempotent");
        }
    }

    // --- AdrStatus::validate_transition rejections ---------------------------

    #[test]
    fn transition_accepted_to_proposed_is_rejected() {
        let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Proposed)
            .expect_err("accepted -> proposed should be rejected");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("accepted"));
        assert!(err.to_string().contains("proposed"));
    }

    #[test]
    fn transition_accepted_to_deleted_is_rejected() {
        let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Deleted)
            .expect_err("accepted -> deleted should be rejected");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("superseded"));
    }

    #[test]
    fn transition_superseded_to_anything_is_rejected() {
        for target in [AdrStatus::Proposed, AdrStatus::Accepted, AdrStatus::Deleted] {
            let err = AdrStatus::validate_transition(AdrStatus::Superseded, target)
                .expect_err("superseded is terminal");
            assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
            assert!(err.to_string().contains("terminal"));
        }
    }

    #[test]
    fn transition_deleted_to_anything_is_rejected() {
        for target in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
        ] {
            let err = AdrStatus::validate_transition(AdrStatus::Deleted, target)
                .expect_err("deleted is terminal");
            assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
            assert!(err.to_string().contains("terminal"));
        }
    }

    // --- AdrStatus serde round-trip -----------------------------------------

    #[test]
    fn adr_status_serde_yaml_round_trip_for_each_variant() {
        for status in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
            AdrStatus::Deleted,
        ] {
            let yaml = serde_yaml::to_string(&status).expect("serialize");
            let round: AdrStatus = serde_yaml::from_str(&yaml).expect("deserialize");
            assert_eq!(round, status);
        }
    }

    // --- LegacyValidation default -------------------------------------------

    #[test]
    fn legacy_validation_default_is_none() {
        assert_eq!(LegacyValidation::default(), LegacyValidation::None);
    }

    // --- Adr serde round-trip -----------------------------------------------

    #[test]
    fn adr_yaml_round_trip_full_struct() {
        let adr = Adr {
            id: "ADR-0042".to_string(),
            title: "Use BLAKE3 for dedup".to_string(),
            status: AdrStatus::Accepted,
            owner: "claude".to_string(),
            created_at: ts(2026, 5, 11),
            accepted_at: Some(ts(2026, 5, 12)),
            last_updated: ts(2026, 5, 12),
            related_features: vec!["knowledge-graph".to_string()],
            related_tasks: vec!["T20260511-1".to_string()],
            tags: vec!["schema".to_string(), "cross-cutting".to_string()],
            paths: vec!["crates/orbit-store/**".to_string()],
            supersedes: vec!["ADR-0001".to_string()],
            superseded_by: None,
            legacy_ids: vec![
                "activity-job/ADR-017".to_string(),
                "activity-job/ADR-018".to_string(),
            ],
            validation_warnings: vec!["missing owner in source".to_string()],
            legacy_validation: LegacyValidation::Warned,
        };

        let yaml = serde_yaml::to_string(&adr).expect("serialize");
        let round: Adr = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(round, adr);
    }

    #[test]
    fn adr_yaml_round_trip_with_missing_optional_fields() {
        let yaml = r#"id: ADR-0001
title: Initial decision
status: proposed
owner: claude
created_at: 2026-05-11T00:00:00Z
last_updated: 2026-05-11T00:00:00Z
"#;
        let adr: Adr = serde_yaml::from_str(yaml).expect("deserialize");
        assert_eq!(adr.id, "ADR-0001");
        assert_eq!(adr.status, AdrStatus::Proposed);
        assert!(adr.accepted_at.is_none());
        assert!(adr.superseded_by.is_none());
        assert!(adr.related_features.is_empty());
        assert!(adr.related_tasks.is_empty());
        assert!(adr.tags.is_empty());
        assert!(adr.paths.is_empty());
        assert!(adr.supersedes.is_empty());
        assert!(adr.legacy_ids.is_empty());
        assert!(adr.validation_warnings.is_empty());
        assert_eq!(adr.legacy_validation, LegacyValidation::None);
    }

    // --- validate_adr_id ----------------------------------------------------

    #[test]
    fn validate_adr_id_accepts_canonical_ids() {
        validate_adr_id("ADR-0001").expect("ADR-0001 should be valid");
        validate_adr_id("ADR-9999").expect("ADR-9999 should be valid");
        validate_adr_id("ADR-12345").expect("ADR-12345 (5 digits) should be valid");
    }

    #[test]
    fn validate_adr_id_rejects_invalid_ids() {
        assert!(validate_adr_id("").is_err(), "empty should be rejected");
        assert!(
            validate_adr_id("ADR-1").is_err(),
            "1 digit should be rejected"
        );
        assert!(
            validate_adr_id("ADR-001").is_err(),
            "3 digits should be rejected"
        );
        assert!(
            validate_adr_id("adr-0001").is_err(),
            "lowercase prefix should be rejected"
        );
        assert!(
            validate_adr_id("ADR-XXXX").is_err(),
            "non-digit suffix should be rejected"
        );
    }

    // --- legacy_id_for ------------------------------------------------------

    #[test]
    fn legacy_id_for_pads_local_number_to_three_digits() {
        assert_eq!(legacy_id_for("activity-job", 17), "activity-job/ADR-017");
    }
}

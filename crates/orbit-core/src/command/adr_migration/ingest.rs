//! Pass-two: allocate global IDs (idempotently), write artifacts via the
//! same store accessors the tool handlers use, then resolve cross-feature
//! supersession edges.
//!
//! Writes go through `runtime.stores().adrs()` which is the same handle
//! `orbit_tool_host::adr_tools` consumes — the lifecycle rules and audit
//! emission are identical to what `orbit.adr.add` / `orbit.adr.update`
//! produce when invoked via the tool surface.

use std::collections::HashMap;

use orbit_common::types::{AdrStatus, LegacyValidation, OrbitError};
use orbit_store::{AdrCreateParams, AdrDocumentUpdateParams};

use super::parse::{EntryKind, ParsedAdrEntry, ParsedStatus};
use super::{CreatedRecord, RollupRecord, SkippedRecord, SupersedeRecord, ValidationWarningRecord};
use crate::OrbitRuntime;

/// Outcome of a single ingest run. The `id_map` is consumed by the sweep
/// phase to rewrite local references.
#[derive(Debug, Default)]
pub(super) struct IngestOutcome {
    /// `(feature, legacy_id)` → global ADR ID, populated for every entry the
    /// migration touched (newly created OR already-in-store skips).
    pub id_map: HashMap<(String, String), String>,
    pub created: Vec<CreatedRecord>,
    pub skipped: Vec<SkippedRecord>,
    pub validation_warnings: Vec<ValidationWarningRecord>,
    pub rollups: Vec<RollupRecord>,
    pub supersedes: Vec<SupersedeRecord>,
}

pub(super) fn ingest(
    runtime: &OrbitRuntime,
    entries: &[ParsedAdrEntry],
    dry_run: bool,
) -> Result<IngestOutcome, OrbitError> {
    let mut outcome = IngestOutcome::default();

    // Build the legacy-ids index from existing artifacts for idempotency.
    let existing_index = if dry_run {
        HashMap::new()
    } else {
        build_existing_index(runtime)?
    };

    // First sub-pass: create/update standalone + rollup entries, populate id_map.
    for entry in entries {
        match &entry.kind {
            EntryKind::Folded { target_legacy } => {
                // Folded entries do not produce artifacts — the rollup target
                // claims their legacy IDs. The id_map still gets populated so
                // the sweep can resolve the folded ID to the rollup's global
                // ID.
                if let Some(global) = outcome
                    .id_map
                    .get(&(entry.feature.clone(), target_legacy.clone()))
                {
                    outcome.id_map.insert(
                        (entry.feature.clone(), entry.legacy_id.clone()),
                        global.clone(),
                    );
                }
            }
            _ => {
                ingest_entry(runtime, entry, &existing_index, dry_run, &mut outcome)?;
            }
        }
    }

    // Second sub-pass: fill in id_map for folded entries that were processed
    // before their rollup target. (First pass is order-dependent.)
    for entry in entries {
        if let EntryKind::Folded { target_legacy } = &entry.kind
            && let Some(global) = outcome
                .id_map
                .get(&(entry.feature.clone(), target_legacy.clone()))
        {
            outcome.id_map.insert(
                (entry.feature.clone(), entry.legacy_id.clone()),
                global.clone(),
            );
        }
    }

    // Third sub-pass: write real supersession edges (non-folded) now that the
    // id_map is fully populated. In dry-run we record the predicted edges
    // without calling the store, so the report previews what the real run
    // would do.
    for entry in entries {
        if let ParsedStatus::SupersededBy {
            target_legacy,
            folded: false,
        } = &entry.status
        {
            let old_global = outcome
                .id_map
                .get(&(entry.feature.clone(), entry.legacy_id.clone()))
                .cloned();
            let new_global = outcome
                .id_map
                .get(&(entry.feature.clone(), target_legacy.clone()))
                .cloned();
            if let (Some(old_id), Some(new_id)) = (old_global, new_global) {
                if old_id == new_id {
                    continue;
                }
                if dry_run {
                    outcome.supersedes.push(SupersedeRecord {
                        old_global_id: old_id,
                        new_global_id: new_id,
                        source_path: entry.source_path.clone(),
                    });
                    continue;
                }
                // The supersession target must be accepted before the
                // supersede edge is permitted.
                let adrs = runtime.stores().adrs();
                if let Some(target) = adrs.get(&new_id)?
                    && target.status == AdrStatus::Proposed
                {
                    adrs.update_status(&new_id, AdrStatus::Accepted)?;
                }
                match adrs.supersede(&old_id, &new_id) {
                    Ok(()) => outcome.supersedes.push(SupersedeRecord {
                        old_global_id: old_id,
                        new_global_id: new_id,
                        source_path: entry.source_path.clone(),
                    }),
                    Err(OrbitError::AdrInvalidTransition(_)) => {
                        // Already superseded or otherwise resolved — idempotent.
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    Ok(outcome)
}

fn build_existing_index(runtime: &OrbitRuntime) -> Result<HashMap<String, String>, OrbitError> {
    let adrs = runtime.stores().adrs();
    let mut index = HashMap::new();
    for adr in adrs.list_filtered(None, None, None, None, None, None)? {
        for legacy in adr.legacy_ids {
            index.insert(legacy, adr.id.clone());
        }
    }
    Ok(index)
}

fn ingest_entry(
    runtime: &OrbitRuntime,
    entry: &ParsedAdrEntry,
    existing_index: &HashMap<String, String>,
    dry_run: bool,
    outcome: &mut IngestOutcome,
) -> Result<(), OrbitError> {
    let legacy_path = format!("{}/{}", entry.feature, entry.legacy_id);
    let mut legacy_ids = vec![legacy_path.clone()];
    if let EntryKind::Rollup { folded_legacy_ids } = &entry.kind {
        for folded in folded_legacy_ids {
            legacy_ids.push(format!("{}/{}", entry.feature, folded));
        }
    }

    // Idempotency: skip if any of this entry's legacy IDs is already mapped.
    for legacy in &legacy_ids {
        if let Some(global) = existing_index.get(legacy) {
            outcome.id_map.insert(
                (entry.feature.clone(), entry.legacy_id.clone()),
                global.clone(),
            );
            if let EntryKind::Rollup { folded_legacy_ids } = &entry.kind {
                for folded in folded_legacy_ids {
                    outcome
                        .id_map
                        .insert((entry.feature.clone(), folded.clone()), global.clone());
                }
            }
            outcome.skipped.push(SkippedRecord {
                global_id: global.clone(),
                legacy_id: legacy.clone(),
                source_path: entry.source_path.clone(),
                reason: format!("already migrated as `{global}`"),
            });
            return Ok(());
        }
    }

    if dry_run {
        // Dry-run: synthesize a placeholder global ID without writing.
        let global = format!("ADR-DRYRUN-{}-{}", entry.feature, entry.legacy_id);
        record_creation(entry, &global, &legacy_ids, outcome);
        return Ok(());
    }

    let adrs = runtime.stores().adrs();
    let body = render_body(entry);
    let owner = infer_owner(entry);

    let created = adrs.add(AdrCreateParams {
        title: entry.title.clone(),
        owner,
        related_features: vec![entry.feature.clone()],
        related_tasks: entry.tasks.clone(),
        body,
    })?;
    let global_id = created.id.clone();

    let legacy_validation = if entry.validation_warnings.is_empty() {
        LegacyValidation::None
    } else {
        LegacyValidation::Warned
    };
    adrs.update_document(
        &global_id,
        &AdrDocumentUpdateParams {
            legacy_ids: Some(legacy_ids.clone()),
            validation_warnings: Some(entry.validation_warnings.clone()),
            legacy_validation: Some(legacy_validation),
            ..Default::default()
        },
    )?;

    // Promote to accepted when the source asserted it (and the entry isn't a
    // folded one — those produce no artifact).
    let target_status = match &entry.status {
        ParsedStatus::Accepted => Some(AdrStatus::Accepted),
        ParsedStatus::SupersededBy { folded: false, .. } => Some(AdrStatus::Accepted),
        ParsedStatus::Proposed => None,
        ParsedStatus::SupersededBy { folded: true, .. } => None,
    };
    if let Some(status) = target_status
        && status == AdrStatus::Accepted
    {
        // Migration must satisfy the proposed→accepted "non-empty related_tasks"
        // rule. If the source entry had no task citations, fall back to a
        // legacy-id token to keep the artifact promotable; document the
        // exception in validation_warnings.
        if entry.tasks.is_empty() {
            let legacy_token = format!("legacy:{legacy_path}");
            let mut warnings = entry.validation_warnings.clone();
            warnings.push(
                "accepted in source but no task IDs cited; backfilled legacy token".to_string(),
            );
            adrs.update_document(
                &global_id,
                &AdrDocumentUpdateParams {
                    related_tasks: Some(vec![legacy_token]),
                    validation_warnings: Some(warnings),
                    legacy_validation: Some(LegacyValidation::Warned),
                    ..Default::default()
                },
            )?;
        }
        adrs.update_status(&global_id, AdrStatus::Accepted)?;
    }

    record_creation(entry, &global_id, &legacy_ids, outcome);
    Ok(())
}

fn record_creation(
    entry: &ParsedAdrEntry,
    global_id: &str,
    legacy_ids: &[String],
    outcome: &mut IngestOutcome,
) {
    outcome.id_map.insert(
        (entry.feature.clone(), entry.legacy_id.clone()),
        global_id.to_string(),
    );
    if let EntryKind::Rollup { folded_legacy_ids } = &entry.kind {
        for folded in folded_legacy_ids {
            outcome.id_map.insert(
                (entry.feature.clone(), folded.clone()),
                global_id.to_string(),
            );
        }
        outcome.rollups.push(RollupRecord {
            global_id: global_id.to_string(),
            rollup_legacy_id: format!("{}/{}", entry.feature, entry.legacy_id),
            source_path: entry.source_path.clone(),
            folded_legacy_ids: folded_legacy_ids
                .iter()
                .map(|id| format!("{}/{}", entry.feature, id))
                .collect(),
        });
    }
    outcome.created.push(CreatedRecord {
        global_id: global_id.to_string(),
        legacy_ids: legacy_ids.to_vec(),
        source_path: entry.source_path.clone(),
        title: entry.title.clone(),
    });
    if !entry.validation_warnings.is_empty() {
        outcome.validation_warnings.push(ValidationWarningRecord {
            global_id: global_id.to_string(),
            legacy_id: format!("{}/{}", entry.feature, entry.legacy_id),
            source_path: entry.source_path.clone(),
            warnings: entry.validation_warnings.clone(),
        });
    }
}

fn render_body(entry: &ParsedAdrEntry) -> String {
    let mut body = String::new();
    body.push_str("## Context\n");
    if entry.context.trim().is_empty() {
        body.push_str("_(migration: missing in source)_\n");
    } else {
        body.push_str(entry.context.trim_end());
        body.push('\n');
    }
    body.push_str("\n## Decision\n");
    if entry.decision.trim().is_empty() {
        body.push_str("_(migration: missing in source)_\n");
    } else {
        body.push_str(entry.decision.trim_end());
        body.push('\n');
    }
    body.push_str("\n## Consequences\n");
    if entry.consequences.trim().is_empty() {
        body.push_str("- Cost: _(migration: missing in source)_\n");
    } else {
        body.push_str(entry.consequences.trim_end());
        body.push('\n');
    }
    body
}

fn infer_owner(entry: &ParsedAdrEntry) -> String {
    // No author metadata in source markdown; use the feature as a stand-in for
    // legacy ownership. Owners can rewrite this post-migration via
    // `orbit.adr.update --owner`.
    format!("legacy:{}", entry.feature)
}

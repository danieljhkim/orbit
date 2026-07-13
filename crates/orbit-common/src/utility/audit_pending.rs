//! Durable per-blob pending-publication root shared by the audit writer paths
//! and the audit garbage collector (ORB-10186).
//!
//! The workspace audit writer/GC guard ([`super::audit_writer_guard`]) makes
//! each *individual* write atomic against the collector, but a content-addressed
//! blob and the envelope/loop event that references it are published by two
//! *separate* guarded calls (`write_blob`, then `write_envelope`/`emit`). The
//! guard is released between them, so the collector could sweep the
//! just-written—but not-yet-referenced—blob and leave the later-published
//! reference dangling.
//!
//! This module closes that split transaction with a durable pending root: when a
//! writer publishes a blob it records a marker `<audit_root>/pending/<hash>`
//! (under the guard, atomic with the blob), and clears it once the referencing
//! envelope/event row is persisted (again under the guard, after the row
//! commits). At no guarded instant is the blob both unmarked *and* unreferenced.
//!
//! The collector treats a *fresh* pending marker as a retained reference, so a
//! blob inside its publication window is never a sweep candidate and `apply`
//! fails closed on it. Markers whose recorded timestamp predates the retention
//! cutoff are stale — their publication window has closed — and are swept as
//! ordinary GC candidates, so an orphaned or crashed-mid-publish blob is still
//! reclaimed, bounded by the retention window rather than leaking forever.
//! Restart-safe: markers are plain files that survive a crash, and each is
//! written atomically (temp + rename) so a crash never leaves a torn marker.
//!
//! The pending directory sits beside `blobs/`, `v2_loop/`, and the guard file,
//! so none of the collector's other scans (blobs, JSONL, holds/exports) mistake
//! a marker for evidence.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::fs::{atomic_write_text, remove_path_if_exists};
use crate::types::OrbitError;

/// Directory beneath the workspace audit root that holds pending-publication
/// markers, one file per in-flight blob hash.
const PENDING_DIR_NAME: &str = "pending";

/// A pending-publication marker recovered from disk.
#[derive(Debug, Clone)]
pub struct PendingMarker {
    /// Content-addressed blob hash this marker protects (its file name).
    pub hash: String,
    /// Absolute path of the marker file.
    pub path: PathBuf,
    /// Recorded publication time, or `None` if the marker is malformed
    /// (corrupt/torn) and its age cannot be trusted.
    pub created: Option<DateTime<Utc>>,
}

/// The pending-marker directory for a workspace audit root. Both the writer
/// paths and the collector derive it identically so they rendezvous on the same
/// files.
pub fn pending_dir(audit_root: &Path) -> PathBuf {
    audit_root.join(PENDING_DIR_NAME)
}

fn marker_path(audit_root: &Path, hash: &str) -> PathBuf {
    pending_dir(audit_root).join(hash)
}

/// Record a pending-publication marker for `hash`, stamped with `now`. Called by
/// a writer under the audit writer/GC guard, immediately after the blob is
/// published, so the marker becomes visible atomically with the blob. Idempotent
/// — a repeat write refreshes the timestamp, reopening the publication window
/// for content that is being republished. A non-hash token (e.g. an
/// `error:`-prefixed write failure) is ignored so no bogus marker is created.
pub fn mark(audit_root: &Path, hash: &str, now: DateTime<Utc>) -> Result<(), OrbitError> {
    if !is_blob_hash(hash) {
        return Ok(());
    }
    atomic_write_text(&marker_path(audit_root, hash), &now.to_rfc3339())?;
    Ok(())
}

/// Remove the pending marker for `hash` if present. Called by a writer under the
/// guard once the referencing row is persisted (the blob is now reachable from
/// retained evidence), and by the collector when it sweeps a stale marker.
/// Absent markers are not an error.
pub fn clear(audit_root: &Path, hash: &str) -> Result<(), OrbitError> {
    remove_path_if_exists(&marker_path(audit_root, hash))?;
    Ok(())
}

/// Clear the pending markers for every blob hash referenced by `payload` (a
/// serialized envelope/loop-event body). Called under the guard right after the
/// referencing row is persisted, so a published reference always retires its
/// blob's marker. Best-effort per hash: a failure to remove one marker leaves it
/// to be stale-swept later and never blocks publication.
pub fn clear_published(audit_root: &Path, payload: &str) {
    for_each_blob_hash(payload, |hash| {
        let _ = clear(audit_root, &hash);
    });
}

/// List every pending marker under `audit_root`. Returns an empty list when the
/// directory does not exist. Non-marker files (e.g. in-flight `*.tmp` staging
/// files, whose names are not bare blob hashes) are skipped.
pub fn list(audit_root: &Path) -> Result<Vec<PendingMarker>, OrbitError> {
    let dir = pending_dir(audit_root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut markers = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !is_blob_hash(name) {
            continue;
        }
        let created = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| DateTime::parse_from_rfc3339(raw.trim()).ok())
            .map(|ts| ts.with_timezone(&Utc));
        markers.push(PendingMarker {
            hash: name.to_string(),
            path,
            created,
        });
    }
    Ok(markers)
}

/// True when `value` is a lowercase-or-uppercase 64-hex sha256 blob hash.
pub fn is_blob_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Invoke `f` with each distinct-position blob hash token in `text`, lowercased.
/// The single tokenizer both the writer (marker clearing) and the collector
/// (reference marking) use, so the two sides can never diverge on what counts as
/// a blob reference.
pub fn for_each_blob_hash(text: &str, mut f: impl FnMut(String)) {
    for token in text.split(|ch: char| !ch.is_ascii_hexdigit()) {
        if is_blob_hash(token) {
            f(token.to_ascii_lowercase());
        }
    }
}

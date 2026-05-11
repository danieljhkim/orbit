//! Parse `docs/design/<feature>/4_decisions.md` into [`ParsedAdrEntry`] records.
//!
//! The parser is intentionally lenient: every entry succeeds and accumulates
//! validation warnings rather than aborting the run. See ADR-011 in
//! `docs/design/adr-artifact/4_decisions.md`.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

/// One ADR heading parsed out of `4_decisions.md`. The `kind` is populated
/// during rollup resolution (`rollup.rs`); the parser produces every entry as
/// [`EntryKind::Unknown`].
#[derive(Debug, Clone)]
pub struct ParsedAdrEntry {
    pub feature: String,
    pub legacy_id: String,
    pub title: String,
    pub status: ParsedStatus,
    pub tasks: Vec<String>,
    pub context: String,
    pub decision: String,
    pub consequences: String,
    pub validation_warnings: Vec<String>,
    pub source_path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedStatus {
    Proposed,
    Accepted,
    SupersededBy { target_legacy: String, folded: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Unknown,
    Standalone,
    Rollup { folded_legacy_ids: Vec<String> },
    Folded { target_legacy: String },
}

/// Walks every `docs/design/<feature>/4_decisions.md` under `design_root` and
/// returns all parsed entries.
pub fn parse_corpus(design_root: &Path) -> Result<Vec<ParsedAdrEntry>, OrbitError> {
    let mut entries = Vec::new();
    if !design_root.is_dir() {
        return Ok(entries);
    }

    let mut feature_dirs: Vec<_> = fs::read_dir(design_root)
        .map_err(|e| OrbitError::Io(format!("read {}: {e}", design_root.display())))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect();
    feature_dirs.sort();

    for feature_dir in feature_dirs {
        let decisions = feature_dir.join("4_decisions.md");
        if !decisions.is_file() {
            continue;
        }
        let feature = feature_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        if feature.is_empty() {
            continue;
        }
        let parsed = parse_file(&decisions, &feature)?;
        entries.extend(parsed);
    }

    Ok(entries)
}

/// Parses a single `4_decisions.md` file. Public for unit-testing.
pub fn parse_file(path: &Path, feature: &str) -> Result<Vec<ParsedAdrEntry>, OrbitError> {
    let raw = fs::read_to_string(path)
        .map_err(|e| OrbitError::Io(format!("read {}: {e}", path.display())))?;
    Ok(parse_string(&raw, feature, path))
}

fn parse_string(raw: &str, feature: &str, source_path: &Path) -> Vec<ParsedAdrEntry> {
    let lines: Vec<&str> = raw.lines().collect();
    let mut entries = Vec::new();

    let mut heading_indices: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_adr_heading(line) {
            heading_indices.push(idx);
        }
    }
    heading_indices.push(lines.len());

    for window in heading_indices.windows(2) {
        let start = window[0];
        let end = window[1];
        let heading = lines[start];
        let body_lines = &lines[start + 1..end];
        let (legacy_id, title) = match split_adr_heading(heading) {
            Some(value) => value,
            None => continue,
        };

        let mut warnings = Vec::new();
        let status_line = find_status_line(body_lines);
        let mut status = match status_line {
            Some(line) => parse_status(line),
            None => {
                warnings.push("missing Status line".to_string());
                ParsedStatus::Proposed
            }
        };
        // CONVENTIONS §4 puts supersession inline on the Status line, but
        // knowledge-graph and task-sync ADRs use `**Status:** Accepted` paired
        // with a separate `**Superseded by:** ADR-NNN` bullet. If the Status
        // line resolved to a non-supersession state and a bullet carries an
        // ADR target, the bullet wins.
        if matches!(status, ParsedStatus::Accepted | ParsedStatus::Proposed)
            && let Some(bullet) = find_superseded_by_bullet(body_lines)
            && let Some(target_legacy) = parse_superseded_by_bullet(bullet)
        {
            status = ParsedStatus::SupersededBy {
                target_legacy,
                folded: false,
            };
        }
        let tasks = status_line.map(extract_task_ids).unwrap_or_default();

        let (context, decision, consequences) = extract_sections(body_lines);
        if context.trim().is_empty() {
            warnings.push("missing Context section".to_string());
        }
        if decision.trim().is_empty() {
            warnings.push("missing Decision section".to_string());
        }
        if consequences.trim().is_empty() {
            warnings.push("missing Consequences section".to_string());
        } else if !consequences_has_cost_bullet(&consequences) {
            warnings.push("Consequences missing labeled Cost bullet".to_string());
        }

        entries.push(ParsedAdrEntry {
            feature: feature.to_string(),
            legacy_id,
            title,
            status,
            tasks,
            context,
            decision,
            consequences,
            validation_warnings: warnings,
            source_path: source_path.to_path_buf(),
            kind: EntryKind::Unknown,
        });
    }

    entries
}

fn is_adr_heading(line: &str) -> bool {
    line.starts_with("## ADR-") && line.contains(" — ")
}

fn split_adr_heading(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start_matches('#').trim();
    let (id_part, title_part) = trimmed.split_once(" — ")?;
    let legacy_id = id_part.trim().to_string();
    let title = title_part.trim().to_string();
    if !legacy_id.starts_with("ADR-") {
        return None;
    }
    Some((legacy_id, title))
}

fn find_status_line<'a>(body: &'a [&'a str]) -> Option<&'a str> {
    body.iter()
        .find(|line| line.trim_start().starts_with("**Status:**"))
        .copied()
}

fn parse_status(line: &str) -> ParsedStatus {
    // Strip the **Status:** prefix and the leading marker.
    let payload = line.trim_start().trim_start_matches("**Status:**").trim();

    // Look at the first segment (separated by " · " or end-of-line).
    let first = payload.split(" · ").next().unwrap_or("").trim();

    // Prefix matching is intentional: corpus entries write things like
    // `Accepted (with open question)` and `Proposed (cross-machine render
    // deferred from minimal Phase 1)`. Check supersession first so the
    // "Superseded by ..." prefix beats the "Superseded" / "Sup..." branches.
    if let Some(rest) = first.strip_prefix("Superseded by ") {
        let folded = rest.contains("(folded)");
        let target_legacy = rest.replace("(folded)", "").trim().to_string();
        ParsedStatus::SupersededBy {
            target_legacy,
            folded,
        }
    } else if first.starts_with("Accepted") {
        ParsedStatus::Accepted
    } else {
        // "Proposed", "Proposed (...)", or any other unrecognized form all
        // fall back to Proposed — the lenient default.
        ParsedStatus::Proposed
    }
}

fn find_superseded_by_bullet<'a>(body: &'a [&'a str]) -> Option<&'a str> {
    body.iter()
        .find(|line| line.trim_start().starts_with("**Superseded by:**"))
        .copied()
}

/// Extracts the supersession target from a `**Superseded by:** ADR-NNN ...`
/// bullet line. Returns `None` if the first token is not an ADR id (e.g.
/// `**Superseded by:** [T20260506-11] for ...`).
fn parse_superseded_by_bullet(line: &str) -> Option<String> {
    let payload = line
        .trim_start()
        .trim_start_matches("**Superseded by:**")
        .trim();
    let token = payload.split_whitespace().next()?;
    let cleaned = token.trim_end_matches(['.', ',', '/']);
    if cleaned.starts_with("ADR-") {
        Some(cleaned.to_string())
    } else {
        None
    }
}

fn extract_task_ids(line: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'T' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b']' {
                end += 1;
            }
            if end < bytes.len() {
                let candidate = &line[start..end];
                if candidate.starts_with('T') && candidate.len() > 1 {
                    ids.push(candidate.to_string());
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    ids
}

fn extract_sections(body: &[&str]) -> (String, String, String) {
    let mut context = String::new();
    let mut decision = String::new();
    let mut consequences = String::new();

    let mut current: Option<&mut String> = None;
    for line in body {
        let stripped = line.trim_start();
        if let Some((section, remainder)) = detect_section_heading(stripped) {
            current = match section {
                Section::Context => Some(&mut context),
                Section::Decision => Some(&mut decision),
                Section::Consequences => Some(&mut consequences),
            };
            // Some entries write `**Context.** <prose>` on a single line —
            // capture the remainder so the body actually populates.
            let trimmed_remainder = remainder.trim();
            if !trimmed_remainder.is_empty()
                && let Some(slot) = current.as_deref_mut()
            {
                slot.push_str(trimmed_remainder);
                slot.push('\n');
            }
            continue;
        }
        if stripped.starts_with("**Status:**") {
            continue;
        }
        if let Some(slot) = current.as_deref_mut() {
            slot.push_str(line);
            slot.push('\n');
        }
    }

    (
        trim_trailing_blank(&context),
        trim_trailing_blank(&decision),
        trim_trailing_blank(&consequences),
    )
}

#[derive(Clone, Copy)]
enum Section {
    Context,
    Decision,
    Consequences,
}

fn detect_section_heading(line: &str) -> Option<(Section, &str)> {
    for (marker, section) in [
        ("**Context.**", Section::Context),
        ("**Decision.**", Section::Decision),
        ("**Consequences.**", Section::Consequences),
        ("## Context", Section::Context),
        ("## Decision", Section::Decision),
        ("## Consequences", Section::Consequences),
    ] {
        if let Some(remainder) = line.strip_prefix(marker) {
            return Some((section, remainder));
        }
    }
    None
}

fn trim_trailing_blank(text: &str) -> String {
    text.trim_end_matches(|c: char| c == '\n' || c.is_whitespace())
        .to_string()
}

fn consequences_has_cost_bullet(consequences: &str) -> bool {
    for line in consequences.lines() {
        let trimmed = line.trim_start();
        if (trimmed.starts_with("- ") || trimmed.starts_with("* "))
            && trimmed
                .trim_start_matches(|c: char| c == '-' || c == '*' || c.is_whitespace())
                .starts_with("Cost:")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_path() -> PathBuf {
        PathBuf::from("docs/design/feature/4_decisions.md")
    }

    #[test]
    fn parses_single_accepted_adr_with_task_ids() {
        let src = "\
## ADR-001 — A decision

**Status:** Accepted · 2026-05 · [T20260427-34], [T20260428-9]

**Context.** A context paragraph.

**Decision.** A decision paragraph.

**Consequences.**
- A consequence.
- Cost: explicit tradeoff.
";
        let entries = parse_string(src, "feature", &make_path());
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.legacy_id, "ADR-001");
        assert_eq!(entry.title, "A decision");
        assert_eq!(entry.status, ParsedStatus::Accepted);
        assert_eq!(entry.tasks, vec!["T20260427-34", "T20260428-9"]);
        assert!(entry.validation_warnings.is_empty(), "{entry:?}");
        assert!(entry.context.contains("A context paragraph"));
        assert!(entry.decision.contains("A decision paragraph"));
        assert!(entry.consequences.contains("Cost:"));
    }

    #[test]
    fn parses_folded_status_with_target_legacy_id() {
        let src = "\
## ADR-003 — Some folded decision

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260418-2143]

Folded into ADR-001's rollup.
";
        let entries = parse_string(src, "activity-job", &make_path());
        assert_eq!(entries.len(), 1);
        match &entries[0].status {
            ParsedStatus::SupersededBy {
                target_legacy,
                folded,
            } => {
                assert_eq!(target_legacy, "ADR-001");
                assert!(*folded);
            }
            other => panic!("expected SupersededBy, got {other:?}"),
        }
    }

    #[test]
    fn parses_real_supersession_without_folded_marker() {
        let src = "\
## ADR-007 — Replaced by a newer choice

**Status:** Superseded by ADR-042 · 2026-05 · [T20260509-2]

**Context.** Replaced.

**Decision.** Done.

**Consequences.**
- Cost: history retained.
";
        let entries = parse_string(src, "auditability", &make_path());
        match &entries[0].status {
            ParsedStatus::SupersededBy {
                target_legacy,
                folded,
            } => {
                assert_eq!(target_legacy, "ADR-042");
                assert!(!*folded);
            }
            other => panic!("expected SupersededBy, got {other:?}"),
        }
    }

    #[test]
    fn lenient_validation_records_warnings_but_does_not_drop_entry() {
        let src = "\
## ADR-001 — Missing cost bullet

**Status:** Accepted · 2026-05 · [T20260509-1]

**Context.** Some context.

**Decision.** A choice.

**Consequences.**
- A consequence without a labeled cost.
";
        let entries = parse_string(src, "feature", &make_path());
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .validation_warnings
                .iter()
                .any(|w| w.contains("Cost")),
            "expected cost-missing warning: {:?}",
            entries[0].validation_warnings,
        );
    }

    #[test]
    fn lenient_validation_flags_missing_consequences_section() {
        let src = "\
## ADR-042 — No consequences

**Status:** Accepted · 2026-05 · [T20260509-1]

**Context.** ctx.

**Decision.** decision.
";
        let entries = parse_string(src, "activity-job", &make_path());
        let warnings = &entries[0].validation_warnings;
        assert!(
            warnings.iter().any(|w| w.contains("Consequences")),
            "expected Consequences-missing warning: {warnings:?}"
        );
    }

    #[test]
    fn parses_multiple_adrs_in_one_file_preserving_order() {
        let src = "\
## ADR-001 — first

**Status:** Accepted · 2026-05 · [T20260509-1]

**Context.** c1.
**Decision.** d1.
**Consequences.**
- Cost: a.

## ADR-002 — second

**Status:** Accepted · 2026-05 · [T20260509-2]

**Context.** c2.
**Decision.** d2.
**Consequences.**
- Cost: b.
";
        let entries = parse_string(src, "feature", &make_path());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].legacy_id, "ADR-001");
        assert_eq!(entries[1].legacy_id, "ADR-002");
    }

    #[test]
    fn superseded_by_bullet_with_adr_target_overrides_accepted_status() {
        // knowledge-graph/ADR-004 shape: Status says Accepted, a separate
        // bullet carries the supersession target.
        let src = "\
## ADR-004 — Shell out to the `git` CLI instead of an in-process library

**Status:** Accepted · 2026-04 · [T20260421-0528]

**Superseded by:** ADR-029 / [T20260506-11].

**Context.** ctx.

**Decision.** decision.

**Consequences.**
- Cost: per-commit fork overhead.
";
        let entries = parse_string(src, "knowledge-graph", &make_path());
        match &entries[0].status {
            ParsedStatus::SupersededBy {
                target_legacy,
                folded,
            } => {
                assert_eq!(target_legacy, "ADR-029");
                assert!(!*folded);
            }
            other => panic!("expected SupersededBy from bullet, got {other:?}"),
        }
        // Tasks come from the Status line, not the bullet.
        assert_eq!(entries[0].tasks, vec!["T20260421-0528"]);
    }

    #[test]
    fn superseded_by_bullet_with_task_only_does_not_override_status() {
        // knowledge-graph/ADR-010 shape: bullet has no ADR target, just a task.
        let src = "\
## ADR-010 — Orbit-owned symbol-level write operations

**Status:** Proposed · 2026-04 · [T20260421-0543]

**Superseded by:** [T20260506-11] for graph task-attribution preservation.

**Context.** ctx.

**Decision.** decision.

**Consequences.**
- Cost: tradeoff.
";
        let entries = parse_string(src, "knowledge-graph", &make_path());
        assert_eq!(entries[0].status, ParsedStatus::Proposed);
    }

    #[test]
    fn accepted_with_parenthetical_qualifier_still_parses_as_accepted() {
        // knowledge-graph/ADR-005 shape: `Accepted (with open question)`.
        let src = "\
## ADR-005 — Some accepted decision with caveats

**Status:** Accepted (with open question) · 2026-04 · [T20260411-0424]

**Context.** ctx.

**Decision.** decision.

**Consequences.**
- Cost: caveat.
";
        let entries = parse_string(src, "knowledge-graph", &make_path());
        assert_eq!(entries[0].status, ParsedStatus::Accepted);
    }

    #[test]
    fn accepts_h2_style_sections_for_bootstrap_body() {
        // adr-artifact's own bootstrap uses ## Context / ## Decision /
        // ## Consequences instead of **Context.** prefixed paragraphs.
        let src = "\
## ADR-001 — Bootstrap-style sections

**Status:** Proposed · 2026-05 · [T20260510-27]

## Context
ctx body

## Decision
decision body

## Consequences
- Cost: bootstrap-style still works.
";
        let entries = parse_string(src, "adr-artifact", &make_path());
        assert!(entries[0].context.contains("ctx body"));
        assert!(entries[0].decision.contains("decision body"));
        assert!(entries[0].consequences.contains("Cost:"));
        assert!(entries[0].validation_warnings.is_empty());
    }
}

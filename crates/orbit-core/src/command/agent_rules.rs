//! Opt-in injection of Orbit's workflow rules into agent-prompt files
//! (`CLAUDE.md`, `AGENTS.md`) at the workspace root.
//!
//! Triggered by `orbit workspace init --inject-agent-rules`. The rule content
//! lives in `crates/orbit-core/assets/agent-rules.md` as a self-contained
//! fenced block (with start/end markers literally inside the asset). Re-runs
//! replace only the content between the markers; content outside is
//! byte-preserved.

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::atomic_write_text;

/// Asset block embedded at compile time. The asset contains the marker pair
/// literally so a read-render-write round-trip is byte-stable when the asset
/// has not changed.
pub const AGENT_RULES_TEMPLATE: &str = include_str!("../../assets/agent-rules.md");

pub const START_MARKER: &str = "<!-- orbit-managed:start -->";
pub const END_MARKER: &str = "<!-- orbit-managed:end -->";

const TARGET_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionAction {
    Created,
    AppendedBlock,
    ReplacedBlock,
}

#[derive(Debug, Clone)]
pub struct InjectionOutcome {
    pub path: PathBuf,
    pub action: InjectionAction,
}

#[derive(Debug, Clone)]
pub struct InjectAgentRulesResult {
    pub outcomes: Vec<InjectionOutcome>,
}

/// Inject (or refresh) the Orbit rules block into `CLAUDE.md` and `AGENTS.md`
/// at the workspace root. Always uses the embedded `AGENT_RULES_TEMPLATE`.
pub fn inject_agent_rules(workspace_root: &Path) -> Result<InjectAgentRulesResult, OrbitError> {
    let block = normalized_block(AGENT_RULES_TEMPLATE)?;
    let mut outcomes = Vec::with_capacity(TARGET_FILES.len());
    for name in TARGET_FILES {
        let path = workspace_root.join(name);
        let action = apply_to_file(&path, &block)?;
        outcomes.push(InjectionOutcome { path, action });
    }
    Ok(InjectAgentRulesResult { outcomes })
}

/// Normalize the template to a single trailing newline so the file produced
/// from a brand-new write ends cleanly.
fn normalized_block(template: &str) -> Result<String, OrbitError> {
    if !template.contains(START_MARKER) || !template.contains(END_MARKER) {
        return Err(OrbitError::InvalidInput(format!(
            "agent-rules template missing required markers ({START_MARKER} / {END_MARKER})"
        )));
    }
    let trimmed = template.trim_end_matches('\n');
    let mut block = String::with_capacity(trimmed.len() + 1);
    block.push_str(trimmed);
    block.push('\n');
    Ok(block)
}

fn apply_to_file(path: &Path, block: &str) -> Result<InjectionAction, OrbitError> {
    if !path.exists() {
        atomic_write_text(path, block).map_err(|e| OrbitError::Io(e.to_string()))?;
        return Ok(InjectionAction::Created);
    }
    let existing = std::fs::read_to_string(path)
        .map_err(|e| OrbitError::Io(format!("read {}: {e}", path.display())))?;
    let has_start = existing.contains(START_MARKER);
    let has_end = existing.contains(END_MARKER);
    match (has_start, has_end) {
        (false, false) => {
            let mut next = existing.clone();
            if !next.ends_with('\n') {
                next.push('\n');
            }
            // One blank-line separator between prior content and the block.
            next.push('\n');
            next.push_str(block);
            atomic_write_text(path, &next).map_err(|e| OrbitError::Io(e.to_string()))?;
            Ok(InjectionAction::AppendedBlock)
        }
        (true, true) => {
            let next = splice_block(&existing, block, path)?;
            if next == existing {
                // No-op — block already byte-matches; skip the write so file
                // mtime does not change unnecessarily.
                return Ok(InjectionAction::ReplacedBlock);
            }
            atomic_write_text(path, &next).map_err(|e| OrbitError::Io(e.to_string()))?;
            Ok(InjectionAction::ReplacedBlock)
        }
        (true, false) => Err(OrbitError::InvalidInput(format!(
            "{}: contains `{START_MARKER}` without matching `{END_MARKER}` — refusing to write; resolve manually",
            path.display()
        ))),
        (false, true) => Err(OrbitError::InvalidInput(format!(
            "{}: contains `{END_MARKER}` without matching `{START_MARKER}` — refusing to write; resolve manually",
            path.display()
        ))),
    }
}

/// Replace the first marker-bounded span in `existing` with `block`. Caller
/// has already verified both markers are present.
fn splice_block(existing: &str, block: &str, path: &Path) -> Result<String, OrbitError> {
    let start = existing.find(START_MARKER).ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "{}: start marker disappeared between checks",
            path.display()
        ))
    })?;
    let end_idx = existing
        .match_indices(END_MARKER)
        .find(|(idx, _)| *idx > start)
        .map(|(idx, _)| idx + END_MARKER.len())
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "{}: end marker appears before start marker — refusing to write; resolve manually",
                path.display()
            ))
        })?;

    let mut next = String::with_capacity(existing.len() + block.len());
    next.push_str(&existing[..start]);
    let trimmed_block = block.trim_end_matches('\n');
    next.push_str(trimmed_block);
    next.push_str(&existing[end_idx..]);
    if !next.ends_with('\n') {
        next.push('\n');
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read tempfile")
    }

    fn synth_block(body: &str) -> String {
        format!("{START_MARKER}\n{body}\n{END_MARKER}\n")
    }

    #[test]
    fn creates_file_when_absent() {
        let dir = tempdir().expect("tempdir");
        let result = inject_agent_rules(dir.path()).expect("inject");

        assert_eq!(result.outcomes.len(), 2);
        for outcome in &result.outcomes {
            assert_eq!(outcome.action, InjectionAction::Created);
            assert!(
                outcome.path.exists(),
                "{} should exist",
                outcome.path.display()
            );
            let body = read(&outcome.path);
            assert!(body.starts_with(START_MARKER));
            assert!(body.contains(END_MARKER));
            assert!(
                body.ends_with('\n'),
                "file must end with exactly one newline"
            );
            assert!(!body.ends_with("\n\n"), "no double trailing newline");
        }
    }

    #[test]
    fn appends_block_when_no_markers() {
        let dir = tempdir().expect("tempdir");
        let claude = dir.path().join("CLAUDE.md");
        let existing = "# Project rules\n\nUse 4-space indent.\n";
        std::fs::write(&claude, existing).expect("seed");

        let block = synth_block("test body");
        let action = apply_to_file(&claude, &block).expect("apply");
        assert_eq!(action, InjectionAction::AppendedBlock);

        let final_content = read(&claude);
        assert!(
            final_content.starts_with(existing),
            "pre-existing bytes must be preserved verbatim at the head"
        );
        assert!(final_content.contains(START_MARKER));
        assert!(final_content.contains(END_MARKER));
        // One blank-line separator between original content and the block.
        let after_existing = &final_content[existing.len()..];
        assert!(
            after_existing.starts_with('\n'),
            "expected blank-line separator before block, got: {after_existing:?}"
        );
    }

    #[test]
    fn replaces_block_when_markers_present() {
        let dir = tempdir().expect("tempdir");
        let claude = dir.path().join("CLAUDE.md");
        let prefix = "# Existing\n\nKeep me.\n\n";
        let suffix = "\n## Tail\n\nKeep me too.\n";
        let stale_block = synth_block("stale body");
        let initial = format!("{prefix}{stale_block}{suffix}");
        std::fs::write(&claude, &initial).expect("seed");

        let fresh_block = synth_block("fresh body");
        let action = apply_to_file(&claude, &fresh_block).expect("apply");
        assert_eq!(action, InjectionAction::ReplacedBlock);

        let final_content = read(&claude);
        assert!(
            final_content.starts_with(prefix),
            "head must be byte-stable"
        );
        assert!(final_content.ends_with(suffix), "tail must be byte-stable");
        assert!(final_content.contains("fresh body"));
        assert!(!final_content.contains("stale body"));
    }

    #[test]
    fn rejects_malformed_marker_pair_start_only() {
        let dir = tempdir().expect("tempdir");
        let claude = dir.path().join("CLAUDE.md");
        let malformed = format!("# rules\n{START_MARKER}\nbody without end\n");
        std::fs::write(&claude, &malformed).expect("seed");

        let err = apply_to_file(&claude, &synth_block("ignored"))
            .expect_err("must reject malformed marker pair");
        assert!(matches!(err, OrbitError::InvalidInput(_)), "got {err:?}");
        match err {
            OrbitError::InvalidInput(msg) => {
                assert!(msg.contains("CLAUDE.md"), "msg: {msg}");
                assert!(msg.contains(END_MARKER), "msg: {msg}");
            }
            _ => unreachable!(),
        }
        // File must be untouched.
        assert_eq!(read(&claude), malformed);
    }

    #[test]
    fn rejects_malformed_marker_pair_end_only() {
        let dir = tempdir().expect("tempdir");
        let agents = dir.path().join("AGENTS.md");
        let malformed = format!("# rules\nbody {END_MARKER} without start\n");
        std::fs::write(&agents, &malformed).expect("seed");

        let err = apply_to_file(&agents, &synth_block("ignored"))
            .expect_err("must reject malformed marker pair");
        match err {
            OrbitError::InvalidInput(msg) => {
                assert!(msg.contains("AGENTS.md"), "msg: {msg}");
                assert!(msg.contains(START_MARKER), "msg: {msg}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        assert_eq!(read(&agents), malformed);
    }

    #[test]
    fn idempotent_when_template_unchanged() {
        let dir = tempdir().expect("tempdir");
        let first = inject_agent_rules(dir.path()).expect("first");
        let claude_first = read(&first.outcomes[0].path);
        let agents_first = read(&first.outcomes[1].path);

        let second = inject_agent_rules(dir.path()).expect("second");
        let claude_second = read(&second.outcomes[0].path);
        let agents_second = read(&second.outcomes[1].path);

        assert_eq!(
            claude_first, claude_second,
            "CLAUDE.md must be byte-stable across re-runs"
        );
        assert_eq!(
            agents_first, agents_second,
            "AGENTS.md must be byte-stable across re-runs"
        );
    }

    #[test]
    fn real_template_round_trips_byte_stably() {
        // Guards against the asset losing its markers or the trim/append
        // logic drifting from the asset's newline shape.
        let block = normalized_block(AGENT_RULES_TEMPLATE).expect("normalize");
        let dir = tempdir().expect("tempdir");
        let claude = dir.path().join("CLAUDE.md");
        let action_one = apply_to_file(&claude, &block).expect("first");
        assert_eq!(action_one, InjectionAction::Created);
        let after_one = read(&claude);

        let action_two = apply_to_file(&claude, &block).expect("second");
        assert_eq!(action_two, InjectionAction::ReplacedBlock);
        let after_two = read(&claude);
        assert_eq!(after_one, after_two);
    }
}

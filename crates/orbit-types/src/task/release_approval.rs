//! Report-only release-preparation pause vs authorized implementation (ORB-11081).
//!
//! The workspace `release-prep` probe is itself a no-diff report. The canonical
//! `Prepare v<X.Y.Z> release` task it creates must stay non-dispatchable until a
//! human classification or approval rewrites that task's durable mandate to the
//! bounded release diff. Shipping the report-only phase through
//! `task_pr_pipeline` is what turned the expected pause into
//! `nothing to commit` (F2026-08-121 / ORB-10987, ORB-11004).

use super::model::Task;

/// Canonical release-preparation tasks carry this tag while they wait for
/// human classification or approval. Auto-dispatch and `git_commit` refuse
/// the tag; approval handoff removes it only after rewriting the durable
/// mandate to the authorized bounded diff.
pub const AWAITING_RELEASE_APPROVAL_TAG: &str = "awaiting-release-approval";

/// Tag that identifies the canonical `Prepare v<X.Y.Z> release` task.
pub const RELEASE_TASK_TAG: &str = "release";

impl Task {
    /// Whether this task is still the report-only release-preparation pause.
    ///
    /// True when the task carries [`RELEASE_TASK_TAG`] and either
    /// [`AWAITING_RELEASE_APPROVAL_TAG`] or a historical report-only mandate
    /// that forbids the bounded diff (the ORB-10987 / ORB-11004 shape). False
    /// once the durable description and criteria instruct the authorized
    /// implementation and the awaiting tag is gone.
    pub fn awaits_release_approval(&self) -> bool {
        if !self.tags.iter().any(|tag| tag == RELEASE_TASK_TAG) {
            return false;
        }
        if self
            .tags
            .iter()
            .any(|tag| tag == AWAITING_RELEASE_APPROVAL_TAG)
        {
            return true;
        }
        report_only_release_mandate(&self.description, &self.acceptance_criteria)
    }

    /// Operator-facing reason a report-only release task cannot enter a
    /// commit-required delivery tail.
    pub fn release_approval_block_reason(&self) -> String {
        format!(
            "task '{}' is a report-only release-preparation mandate awaiting human classification \
             or approval; rewrite its durable description and acceptance criteria to the authorized \
             bounded diff and remove `{AWAITING_RELEASE_APPROVAL_TAG}` before backlog or \
             in-progress admission",
            self.id
        )
    }
}

fn report_only_release_mandate(description: &str, acceptance_criteria: &[String]) -> bool {
    let blob = std::iter::once(description)
        .chain(acceptance_criteria.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if authorizes_bounded_release_diff(&blob) {
        return false;
    }
    forbids_bounded_release_diff(&blob)
}

fn authorizes_bounded_release_diff(lower: &str) -> bool {
    lower.contains("implement the approved")
}

fn forbids_bounded_release_diff(lower: &str) -> bool {
    let no_commit = lower.contains("do not commit") || lower.contains("must not commit");
    let no_bump_or_changelog = lower.contains("bump versions")
        || lower.contains("edit changelog")
        || lower.contains("edit `changelog.md`");
    let pause = lower.contains("human approval")
        || lower.contains("approval boundary")
        || lower.contains("classification");
    no_commit && (pause || no_bump_or_changelog)
}

use std::fs;

use orbit_types::task::{AWAITING_RELEASE_APPROVAL_TAG, RELEASE_TASK_TAG};
use serde_json::json;

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::git_output;

const REPORT_ONLY: &str = "\
This handoff is intentionally bounded: do not commit, tag, push, publish, \
promote, merge, bump versions, or edit CHANGELOG.md. Stop at the human \
approval boundary.
";

const AUTHORIZED: &str = "\
Implement the approved release-preparation diff according to RELEASING.md.
";

#[test]
fn report_only_release_phase_cannot_reach_git_commit() {
    let temp = initialized_git_repo();
    let workspace = temp.path();

    let mut task = task_with_file(
        "ORB-10987",
        "Prepare v0.14.0 release",
        "CHANGELOG.md",
        "codex",
    );
    task.description = REPORT_ONLY.to_string();
    task.tags = vec![
        RELEASE_TASK_TAG.to_string(),
        AWAITING_RELEASE_APPROVAL_TAG.to_string(),
    ];
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    let error =
        git_commit(&host, &input).expect_err("report-only release phase must not enter git_commit");
    let message = error.to_string();
    assert!(
        message.contains("report-only release-preparation mandate"),
        "{message}"
    );
    assert!(
        !message.contains("nothing to commit"),
        "must not look like an empty implementation: {message}"
    );
    let log = git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count commits");
    assert_eq!(log.trim(), "1", "report-only phase must create no commit");
}

#[test]
fn authorized_release_implementation_phase_can_git_commit() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    fs::write(workspace.join("CHANGELOG.md"), "## 0.15.0\n").unwrap();

    let mut task = task_with_file(
        "ORB-11004",
        "Prepare v0.15.0 release",
        "CHANGELOG.md",
        "codex",
    );
    task.description = AUTHORIZED.to_string();
    task.tags = vec![RELEASE_TASK_TAG.to_string()];
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_sha": git_output(workspace, &["rev-parse", "HEAD"]).expect("base sha"),
    });

    let result = git_commit(&host, &input).expect("authorized implementation phase can commit");
    assert_eq!(result["committed"], json!(true));
    assert_eq!(result["skipped_no_diff_expected"], json!(false));
    let log = git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count commits");
    assert_eq!(log.trim(), "2");
}

#[test]
fn no_diff_probe_still_skips_git_commit_without_failing() {
    let temp = initialized_git_repo();
    let workspace = temp.path();

    let mut task = task_with_file(
        "ORB-PROBE",
        "Check whether the next Orbit release is ready to prepare",
        "CHANGELOG.md",
        "luna",
    );
    task.tags = vec![
        "release-prep".to_string(),
        orbit_types::task::NO_DIFF_EXPECTED_TAG.to_string(),
    ];
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    let result = git_commit(&host, &input).expect("probe no-diff skip succeeds");
    assert_eq!(result["skipped_no_diff_expected"], json!(true));
}

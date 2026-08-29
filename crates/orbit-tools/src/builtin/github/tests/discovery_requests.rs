//! Argv construction and response shaping for the read-only CI discovery
//! tools. These assert the contract a task body depends on without needing a
//! GitHub CLI on the machine running the tests.

use serde_json::json;

use crate::builtin::github::{pr_list, run_list, run_logs, run_view};

#[test]
fn run_list_applies_every_filter_and_caps_the_limit() {
    let req = run_list::build_exec_request(&json!({
        "branch": "release",
        "workflow": "ci.yml",
        "status": "failure",
        "event": "push",
        "limit": 5_000,
        "repo": "owner/name",
    }))
    .expect("request");

    assert_eq!(req.program, "gh");
    assert_eq!(&req.args[..2], &["run".to_string(), "list".to_string()]);
    for expected in [
        "--branch",
        "release",
        "--workflow",
        "ci.yml",
        "--status",
        "failure",
        "--event",
        "push",
        "--repo",
        "owner/name",
    ] {
        assert!(
            req.args.iter().any(|arg| arg == expected),
            "missing {expected} in {:?}",
            req.args
        );
    }
    let limit = req
        .args
        .iter()
        .position(|arg| arg == "--limit")
        .map(|index| req.args[index + 1].clone())
        .expect("limit flag");
    assert_eq!(
        limit, "100",
        "an oversized limit must clamp, not pass through"
    );
}

#[test]
fn run_list_defaults_to_an_unfiltered_bounded_query() {
    let req = run_list::build_exec_request(&json!({})).expect("request");

    assert!(!req.args.iter().any(|arg| arg == "--branch"));
    let limit = req
        .args
        .iter()
        .position(|arg| arg == "--limit")
        .map(|index| req.args[index + 1].clone())
        .expect("limit flag");
    assert_eq!(limit, "20");
}

#[test]
fn a_filter_value_cannot_smuggle_another_gh_flag() {
    let error = run_list::build_exec_request(&json!({ "branch": "--repo" }))
        .expect_err("a flag-shaped filter must be rejected");

    assert!(error.to_string().contains("branch"), "{error}");
}

#[test]
fn run_view_requires_a_numeric_run_id() {
    let error = run_view::build_exec_request(&json!({ "run": "--log" }))
        .expect_err("a non-numeric run must be rejected");

    assert!(error.to_string().contains("run"), "{error}");
}

#[test]
fn run_view_projects_the_reported_head_sha_and_collects_failed_jobs() {
    let projected = run_view::project_run_view(&json!({
        "databaseId": 42,
        "number": 7,
        "workflowName": "CI",
        "headSha": "abc123",
        "event": "pull_request",
        "url": "https://example.invalid/o/r/actions/runs/42",
        "jobs": [
            {
                "databaseId": 1,
                "name": "green",
                "conclusion": "success",
                "steps": [{"number": 1, "name": "run", "conclusion": "success"}],
            },
            {
                "databaseId": 2,
                "name": "red",
                "conclusion": "failure",
                "steps": [
                    {"number": 1, "name": "checkout", "conclusion": "success"},
                    {"number": 2, "name": "test", "conclusion": "failure"},
                ],
            },
        ],
    }));

    assert_eq!(projected["reported_head_sha"], "abc123");
    assert!(
        projected.get("sha").is_none(),
        "the reported head SHA must not also appear under an ambiguous name"
    );
    assert_eq!(projected["jobs"].as_array().expect("jobs").len(), 2);
    let failed = projected["failed_jobs"].as_array().expect("failed jobs");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["name"], "red");
    assert_eq!(
        failed[0]["failed_steps"].as_array().expect("steps").len(),
        1
    );
    assert_eq!(failed[0]["failed_steps"][0]["name"], "test");
    assert_eq!(
        failed[0]["url"], "https://example.invalid/o/r/actions/runs/42/job/2",
        "a failing job must be linkable even though gh reports no job URL"
    );
}

#[test]
fn a_cancelled_job_counts_as_unsuccessful() {
    let projected = run_view::project_run_view(&json!({
        "jobs": [{"databaseId": 1, "name": "flaky", "conclusion": "cancelled", "steps": []}],
    }));

    assert_eq!(projected["failed_jobs"].as_array().expect("jobs").len(), 1);
}

#[test]
fn run_logs_defaults_to_failed_steps_and_accepts_the_full_log() {
    let failed = run_logs::build_exec_request(&json!({ "run": "99" })).expect("request");
    assert!(failed.args.iter().any(|arg| arg == "--log-failed"));
    assert!(!failed.args.iter().any(|arg| arg == "--log"));

    let all = run_logs::build_exec_request(&json!({ "run": "99", "scope": "all", "job": "5" }))
        .expect("request");
    assert!(all.args.iter().any(|arg| arg == "--log"));
    assert!(all.args.iter().any(|arg| arg == "--job"));
    assert!(all.args.iter().any(|arg| arg == "5"));
}

#[test]
fn run_logs_rejects_an_unknown_scope() {
    let error = run_logs::build_exec_request(&json!({ "run": "99", "scope": "everything" }))
        .expect_err("an unknown scope must be rejected");

    assert!(error.to_string().contains("scope"), "{error}");
}

#[test]
fn pr_list_projects_each_head_sha_under_its_own_name() {
    let projected = pr_list::project_pull_request(&json!({
        "number": 3,
        "title": "Fix it",
        "state": "OPEN",
        "isDraft": false,
        "headRefName": "topic",
        "headRefOid": "deadbee",
        "baseRefName": "trunk",
    }));

    assert_eq!(projected["reported_head_sha"], "deadbee");
    assert_eq!(projected["head_branch"], "topic");
    assert!(projected.get("sha").is_none());
}

#[test]
fn pr_list_defaults_to_a_bounded_open_query() {
    let req = pr_list::build_exec_request(&json!({})).expect("request");

    assert_eq!(&req.args[..2], &["pr".to_string(), "list".to_string()]);
    let limit = req
        .args
        .iter()
        .position(|arg| arg == "--limit")
        .map(|index| req.args[index + 1].clone())
        .expect("limit flag");
    assert_eq!(limit, "30");
}

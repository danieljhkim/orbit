use orbit_types::task::TaskStatus;
use serde_json::{Value, json};

use super::super::verify::verify;
use super::support::{FakeQueries, InstantWaiter, TestHost, run, task};

const CANDIDATE: &str = "abcabcabcabcabcabcabcabcabcabcabcabcabca";
const OTHER: &str = "0123456789012345678901234567890123456789";

fn host() -> TestHost {
    TestHost::new(vec![task("ORB-1", TaskStatus::InProgress)])
}

fn input(extra: Value) -> Value {
    let mut base = json!({
        "candidate_sha": CANDIDATE,
        "head_branch": "topic",
        "completed_task_ids": ["ORB-1"],
        "expected_workflows": ["ci"],
        "max_wait_seconds": 0,
    });
    for (key, value) in extra.as_object().expect("object").clone() {
        base[key] = value;
    }
    base
}

#[test]
fn green_candidate_is_promotable_and_writes_nothing() {
    let queries = FakeQueries::authenticated().with_runs(
        "topic",
        vec![vec![
            run(
                1,
                "ci",
                CANDIDATE,
                "completed",
                Some("success"),
                "2026-08-30T01:00:00Z",
            ),
            // An informational workflow counts too: every run on the candidate
            // is affected, required or not.
            run(
                2,
                "docs",
                CANDIDATE,
                "completed",
                Some("success"),
                "2026-08-30T01:00:00Z",
            ),
            // A run on a different commit is not this candidate's business.
            run(
                3,
                "ci",
                OTHER,
                "completed",
                Some("failure"),
                "2026-08-30T00:00:00Z",
            ),
        ]],
    );
    let host = host();

    let result = verify(&host, &queries, &InstantWaiter, &input(json!({}))).expect("verify");

    assert_eq!(result["verdict"], json!("green"));
    assert_eq!(result["promotable"], json!(true));
    assert_eq!(result["observed_workflows"], json!(["ci", "docs"]));
    assert_eq!(result["missing_workflows"], json!([]));
    assert!(
        host.updates().is_empty(),
        "a green candidate writes nothing"
    );
}

#[test]
fn a_red_informational_check_is_not_a_pass() {
    let queries = FakeQueries::authenticated()
        .with_runs(
            "topic",
            vec![vec![
                run(
                    1,
                    "ci",
                    CANDIDATE,
                    "completed",
                    Some("success"),
                    "2026-08-30T01:00:00Z",
                ),
                run(
                    2,
                    "docs",
                    CANDIDATE,
                    "completed",
                    Some("failure"),
                    "2026-08-30T01:00:00Z",
                ),
            ]],
        )
        .with_run_view(
            "2",
            json!({"failed_jobs": [{"job_id": 9, "name": "mdbook", "conclusion": "failure"}]}),
        )
        .with_log("2", false, "docs\tmdbook\tbroken link\n");
    let host = host();

    let error = verify(&host, &queries, &InstantWaiter, &input(json!({})))
        .expect_err("a red candidate must not reach promotion");
    assert!(error.to_string().contains("verdict=red"), "{error}");

    let updates = host.updates();
    assert_eq!(
        updates[0].1.append_comments.len(),
        1,
        "verdict is recorded durably"
    );
    let comment = &updates[0].1.append_comments[0].message;
    assert!(comment.contains("\"verdict\": \"red\""), "{comment}");
    assert!(
        comment.contains("mdbook"),
        "failed jobs travel with the verdict: {comment}"
    );
    assert!(
        comment.contains("broken link"),
        "log excerpt travels too: {comment}"
    );
    assert_eq!(updates[1].1.status, Some(TaskStatus::Blocked));
    assert_eq!(
        updates[1].1.status_event.as_deref(),
        Some("ci_candidate_not_green")
    );
}

#[test]
fn cancelled_is_distinguishable_from_red() {
    let queries = FakeQueries::authenticated().with_runs(
        "topic",
        vec![vec![run(
            1,
            "ci",
            CANDIDATE,
            "completed",
            Some("cancelled"),
            "2026-08-30T01:00:00Z",
        )]],
    );

    let error = verify(&host(), &queries, &InstantWaiter, &input(json!({})))
        .expect_err("cancelled is not a pass");
    assert!(error.to_string().contains("verdict=cancelled"), "{error}");
    assert!(
        !error.to_string().contains("verdict=red"),
        "a run that produced no verdict is not a failing test: {error}"
    );
}

#[test]
fn queued_in_progress_and_missing_are_each_named_when_not_waiting() {
    for (status, expected) in [("queued", "queued"), ("in_progress", "in_progress")] {
        let queries = FakeQueries::authenticated().with_runs(
            "topic",
            vec![vec![run(
                1,
                "ci",
                CANDIDATE,
                status,
                None,
                "2026-08-30T01:00:00Z",
            )]],
        );
        let error = verify(&host(), &queries, &InstantWaiter, &input(json!({})))
            .expect_err("an unsettled candidate is not promotable");
        assert!(
            error.to_string().contains(&format!("verdict={expected}")),
            "{status} must stay distinguishable: {error}"
        );
    }

    // An expected workflow with no run at all is `missing`, never green.
    let queries = FakeQueries::authenticated().with_runs("topic", vec![vec![]]);
    let error = verify(&host(), &queries, &InstantWaiter, &input(json!({})))
        .expect_err("a candidate nothing ran on is not promotable");
    assert!(error.to_string().contains("verdict=missing"), "{error}");
}

#[test]
fn a_wait_that_runs_out_of_budget_is_not_reported_as_a_ci_failure() {
    let queries = FakeQueries::authenticated().with_runs(
        "topic",
        vec![vec![run(
            1,
            "ci",
            CANDIDATE,
            "in_progress",
            None,
            "2026-08-30T01:00:00Z",
        )]],
    );
    let host = host();

    let error = verify(
        &host,
        &queries,
        &InstantWaiter,
        &input(json!({"max_wait_seconds": 60, "poll_interval_seconds": 30})),
    )
    .expect_err("an unverified candidate must not promote");

    let message = error.to_string();
    assert!(message.contains("verdict=wait_timeout"), "{message}");
    assert!(message.contains("pending_state=in_progress"), "{message}");
    assert!(
        message.contains("A wait timeout and a cancelled run are not CI failures"),
        "{message}"
    );
    let comment = &host.updates()[0].1.append_comments[0].message;
    assert!(
        comment.contains("\"verdict\": \"wait_timeout\""),
        "{comment}"
    );
    assert!(comment.contains("\"waited_seconds\": 60"), "{comment}");
    assert!(comment.contains("\"polls\": 3"), "{comment}");
}

#[test]
fn a_candidate_that_goes_green_while_waiting_is_promoted() {
    let queries = FakeQueries::authenticated().with_runs(
        "topic",
        vec![
            vec![run(
                1,
                "ci",
                CANDIDATE,
                "queued",
                None,
                "2026-08-30T01:00:00Z",
            )],
            vec![run(
                1,
                "ci",
                CANDIDATE,
                "completed",
                Some("success"),
                "2026-08-30T01:00:00Z",
            )],
        ],
    );
    let host = host();

    let result = verify(
        &host,
        &queries,
        &InstantWaiter,
        &input(json!({"max_wait_seconds": 300, "poll_interval_seconds": 30})),
    )
    .expect("verify");

    assert_eq!(result["verdict"], json!("green"));
    assert_eq!(result["polls"], json!(2));
    assert_eq!(result["waited_seconds"], json!(30));
    assert!(host.updates().is_empty());
}

#[test]
fn the_latest_run_per_workflow_decides_so_a_rerun_can_clear_a_failure() {
    let queries = FakeQueries::authenticated().with_runs(
        "topic",
        vec![vec![
            run(
                2,
                "ci",
                CANDIDATE,
                "completed",
                Some("success"),
                "2026-08-30T02:00:00Z",
            ),
            run(
                1,
                "ci",
                CANDIDATE,
                "completed",
                Some("failure"),
                "2026-08-30T01:00:00Z",
            ),
        ]],
    );

    let result = verify(&host(), &queries, &InstantWaiter, &input(json!({}))).expect("verify");
    assert_eq!(result["verdict"], json!("green"));
}

#[test]
fn a_candidate_sha_that_is_not_an_exact_commit_is_refused() {
    let queries = FakeQueries::authenticated();
    let error = verify(
        &host(),
        &queries,
        &InstantWaiter,
        &json!({"candidate_sha": "topic", "head_branch": "topic"}),
    )
    .expect_err("a branch name is not a candidate commit");
    assert!(error.to_string().contains("40- or 64-character commit sha"));
}

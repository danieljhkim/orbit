use serde_json::{Value, json};

use super::super::collect::collect;
use super::support::{FakeQueries, run, run_on_branch};

const HEAD: &str = "1111111111111111111111111111111111111111";
const OLD: &str = "2222222222222222222222222222222222222222";

fn input() -> Value {
    json!({"integration_branch": "topic", "max_checkout_log_reads": 1})
}

#[test]
fn unauthenticated_host_stops_before_any_query() {
    let queries = FakeQueries::unauthenticated("gh is present but holds no usable credentials");
    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["collected"], json!(false));
    assert_eq!(evidence["outcome_hint"], json!("capability_unavailable"));
    assert_eq!(evidence["capability"]["authenticated"], json!(false));
    // Nothing that could be misread as "we looked and found nothing".
    assert!(evidence.get("current_failures").is_none());
    assert!(evidence.get("heads").is_none());
    assert!(
        evidence["capability"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("no usable credentials"))
    );
}

#[test]
fn separates_event_sha_current_head_and_actual_checkout_commit() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(vec![vec![run(
            10,
            "ci",
            HEAD,
            "completed",
            Some("failure"),
            "2026-08-30T01:00:00Z",
        )]])
        .with_run_view(
            "10",
            json!({"failed_jobs": [{"job_id": 5, "name": "build", "conclusion": "failure"}]}),
        )
        .with_log("10", false, "ci\tbuild\t2026-08-30T01:00:00Z assertion failed\n")
        .with_log(
            "10",
            true,
            "ci\tCheckout\t2026-08-30T01:00:00Z HEAD is now at 3333333333333333333333333333333333333333\n",
        );

    let evidence = collect(&queries, &input()).expect("collect");
    let failure = &evidence["current_failures"][0];

    assert_eq!(failure["event_reported_head_sha"], json!(HEAD));
    assert_eq!(failure["current_ref_head_sha"], json!(HEAD));
    assert_eq!(
        failure["actual_checkout_shas"],
        json!(["3333333333333333333333333333333333333333"])
    );
    assert_eq!(failure["checkout_evidence_scope"], json!("all"));
    assert_eq!(failure["failed_jobs"][0]["name"], json!("build"));
    assert!(
        failure["log_excerpt"]
            .as_str()
            .is_some_and(|log| log.contains("assertion failed"))
    );
    assert_eq!(evidence["outcome_hint"], json!("current_failures"));
}

#[test]
fn latest_non_failing_run_supersedes_older_failures_across_refs_and_shas() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(vec![vec![
            // The latest CI run is successful on the release branch and a
            // different SHA. Both older CI failures are stale even though the
            // old topic ref still points at one failure's SHA.
            run_on_branch(
                30,
                "ci",
                "main",
                OLD,
                "completed",
                Some("success"),
                "2026-08-30T03:00:00Z",
            ),
            run_on_branch(
                20,
                "ci",
                "topic",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T02:00:00Z",
            ),
            run_on_branch(
                10,
                "ci",
                "old-pr",
                OLD,
                "completed",
                Some("failure"),
                "2026-08-30T01:00:00Z",
            ),
            // A completed non-failing conclusion has the same authority as a
            // success for another workflow.
            run(
                40,
                "docs",
                HEAD,
                "completed",
                Some("skipped"),
                "2026-08-30T04:00:00Z",
            ),
            run(
                35,
                "docs",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T03:30:00Z",
            ),
        ]]);

    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["current_failures"], json!([]));
    assert_eq!(evidence["outcome_hint"], json!("no_current_failure"));
    let reasons: Vec<&str> = evidence["stale_or_superseded"]
        .as_array()
        .expect("stale array")
        .iter()
        .filter_map(|entry| entry["reason"].as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            "superseded_by_newer_workflow_run",
            "superseded_by_newer_workflow_run",
            "superseded_by_newer_workflow_run",
        ]
    );
    let superseding_ids: Vec<u64> = evidence["stale_or_superseded"]
        .as_array()
        .expect("stale array")
        .iter()
        .filter_map(|entry| entry["superseded_by"]["run_id"].as_u64())
        .collect();
    assert_eq!(superseding_ids, [30, 30, 40]);
}

#[test]
fn latest_in_flight_run_does_not_resurrect_an_older_failure() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(vec![vec![
            run(40, "ci", HEAD, "in_progress", None, "2026-08-30T04:00:00Z"),
            run(
                30,
                "ci",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T03:00:00Z",
            ),
            run(50, "lint", HEAD, "queued", None, "2026-08-30T05:00:00Z"),
            run(
                45,
                "lint",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T04:30:00Z",
            ),
        ]]);

    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["current_failures"], json!([]));
    let in_flight_ids: Vec<u64> = evidence["in_flight"]
        .as_array()
        .expect("in flight")
        .iter()
        .filter_map(|run| run["run_id"].as_u64())
        .collect();
    assert_eq!(in_flight_ids, [40, 50]);
    let stale_ids: Vec<u64> = evidence["stale_or_superseded"]
        .as_array()
        .expect("stale")
        .iter()
        .filter_map(|run| run["run_id"].as_u64())
        .collect();
    assert_eq!(stale_ids, [30, 45]);
    assert_eq!(
        evidence["stale_or_superseded"][0]["superseded_by"]["status"],
        json!("in_progress")
    );
}

#[test]
fn old_dependabot_failure_is_superseded_by_newer_repository_ci_run() {
    const DEPENDABOT_SHA: &str = "0f14f3f2ad2c863f902c0add969ff09d10e3f15c";
    const OLD_RUN_ID: u64 = 31_583_558_682;

    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_pull_request(json!({
            "number": 959,
            "url": "https://github.com/acme/orbit/pull/959",
            "head_branch": "dependabot/cargo/old",
            "reported_head_sha": DEPENDABOT_SHA,
        }))
        .with_runs(vec![vec![
            run_on_branch(
                OLD_RUN_ID + 100,
                "CI",
                "main",
                HEAD,
                "completed",
                Some("success"),
                "2026-08-31T02:00:00Z",
            ),
            run_on_branch(
                OLD_RUN_ID,
                "CI",
                "dependabot/cargo/old",
                DEPENDABOT_SHA,
                "completed",
                Some("failure"),
                "2026-08-20T01:00:00Z",
            ),
        ]]);

    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["current_failures"], json!([]));
    assert_eq!(evidence["outcome_hint"], json!("no_current_failure"));
    assert_eq!(
        evidence["stale_or_superseded"][0]["run_id"],
        json!(OLD_RUN_ID)
    );
    assert_eq!(
        evidence["stale_or_superseded"][0]["ref_kind"],
        json!("pull_request")
    );
    assert_eq!(
        evidence["stale_or_superseded"][0]["investigated"],
        json!(false),
        "a superseded run must never consume investigation or filing input: {evidence}"
    );
}

#[test]
fn latest_selection_is_independent_per_workflow_and_breaks_ties_by_run_id() {
    let tied_at = "2026-08-30T05:00:00Z";
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(vec![vec![
            // Higher run ID wins the CI tie and suppresses its failure.
            run(52, "ci", HEAD, "completed", Some("success"), tied_at),
            run(51, "ci", HEAD, "completed", Some("failure"), tied_at),
            // Selecting CI must not hide lint's independently latest run.
            run(61, "lint", HEAD, "completed", Some("failure"), tied_at),
            run(
                60,
                "lint",
                HEAD,
                "completed",
                Some("success"),
                "2026-08-30T04:00:00Z",
            ),
        ]]);

    let evidence = collect(&queries, &input()).expect("collect");

    let current_ids: Vec<u64> = evidence["current_failures"]
        .as_array()
        .expect("current failures")
        .iter()
        .filter_map(|run| run["run_id"].as_u64())
        .collect();
    assert_eq!(current_ids, [61]);
    assert_eq!(evidence["stale_or_superseded"][0]["run_id"], json!(51));
    assert_eq!(
        evidence["stale_or_superseded"][0]["superseded_by"]["run_id"],
        json!(52)
    );
}

#[test]
fn derives_release_head_from_github_and_reports_every_bound_it_hit() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", OLD)
        .with_runs(vec![vec![
            run(
                51,
                "a",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T05:00:00Z",
            ),
            run(
                52,
                "b",
                HEAD,
                "completed",
                Some("failure"),
                "2026-08-30T05:00:00Z",
            ),
        ]]);

    let evidence = collect(
        &queries,
        &json!({"integration_branch": "topic", "max_investigated_runs": 1}),
    )
    .expect("collect");

    let heads = evidence["heads"].as_array().expect("heads");
    assert_eq!(heads.len(), 2);
    assert_eq!(heads[0]["kind"], json!("integration"));
    assert_eq!(heads[0]["branch"], json!("topic"));
    // The release branch is whatever GitHub reports as the default, never a
    // guess from a naming convention.
    assert_eq!(heads[1]["kind"], json!("release"));
    assert_eq!(heads[1]["branch"], json!("main"));
    assert_eq!(heads[1]["current_head_sha"], json!(OLD));

    let truncation = &evidence["truncation"];
    assert_eq!(truncation["current_failures_discovered"], json!(2));
    assert_eq!(truncation["current_failures_investigated"], json!(1));
    assert!(
        truncation["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|note| note
                .as_str()
                .is_some_and(|note| note.contains("not investigated"))),
        "truncation must be reported explicitly: {truncation}"
    );
    assert_eq!(evidence["current_failures"][0]["investigated"], json!(true));
    assert_eq!(
        evidence["current_failures"][1]["investigated"],
        json!(false)
    );
}

#[test]
fn integration_and_release_on_the_same_branch_are_scanned_once() {
    let queries = FakeQueries::authenticated().with_head("main", HEAD);

    let evidence = collect(&queries, &json!({"integration_branch": "main"})).expect("collect");

    assert_eq!(evidence["heads"].as_array().expect("heads").len(), 1);
    assert_eq!(evidence["heads"][0]["kind"], json!("integration"));
}

#[test]
fn empty_failed_step_log_is_recorded_in_query_errors() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(vec![vec![run(
            10,
            "ci",
            HEAD,
            "completed",
            Some("failure"),
            "2026-08-30T01:00:00Z",
        )]])
        .with_run_view(
            "10",
            json!({"failed_jobs": [{"job_id": 5, "name": "build", "conclusion": "failure"}]}),
        );

    let evidence = collect(&queries, &input()).expect("collect");
    let failure = &evidence["current_failures"][0];
    assert_eq!(failure["log_excerpt"], json!(""));
    let errors = evidence["query_errors"].as_array().expect("query_errors");
    assert!(
        errors.iter().any(|error| {
            error["query"] == json!("run_logs")
                && error["run_id"] == json!("10")
                && error["error"]
                    .as_str()
                    .is_some_and(|text| text.contains("no log text"))
        }),
        "empty failed-step log must be a query error, got {errors:?}"
    );
}

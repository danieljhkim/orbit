use serde_json::{Value, json};

use super::super::collect::collect;
use super::support::{FakeQueries, run};

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
        .with_runs(
            "topic",
            vec![vec![run(10, "ci", HEAD, "completed", Some("failure"), "2026-08-30T01:00:00Z")]],
        )
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
fn advanced_head_and_superseding_success_are_stale_not_current() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(
            "topic",
            vec![vec![
                // Superseded: a later success of the same workflow at the same SHA.
                run(
                    30,
                    "ci",
                    HEAD,
                    "completed",
                    Some("success"),
                    "2026-08-30T03:00:00Z",
                ),
                run(
                    20,
                    "ci",
                    HEAD,
                    "completed",
                    Some("failure"),
                    "2026-08-30T02:00:00Z",
                ),
                // Stale: the branch has moved past the commit this run tested.
                run(
                    10,
                    "lint",
                    OLD,
                    "completed",
                    Some("failure"),
                    "2026-08-30T01:00:00Z",
                ),
            ]],
        );

    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["current_failures"], json!([]));
    assert_eq!(evidence["outcome_hint"], json!("no_current_failure"));
    let reasons: Vec<&str> = evidence["stale_or_superseded"]
        .as_array()
        .expect("stale array")
        .iter()
        .filter_map(|entry| entry["reason"].as_str())
        .collect();
    assert_eq!(reasons, ["superseded_by_success", "advanced_head"]);
    assert!(
        evidence["stale_or_superseded"][0]["superseded_by"]["run_id"] == json!(30),
        "supersession must cite the run that supersedes: {evidence}"
    );
}

#[test]
fn queued_runs_at_the_current_head_are_in_flight_not_failures() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", HEAD)
        .with_runs(
            "topic",
            vec![vec![run(
                40,
                "ci",
                HEAD,
                "in_progress",
                None,
                "2026-08-30T04:00:00Z",
            )]],
        );

    let evidence = collect(&queries, &input()).expect("collect");

    assert_eq!(evidence["current_failures"], json!([]));
    assert_eq!(evidence["in_flight"][0]["run_id"], json!(40));
}

#[test]
fn derives_release_head_from_github_and_reports_every_bound_it_hit() {
    let queries = FakeQueries::authenticated()
        .with_head("topic", HEAD)
        .with_head("main", OLD)
        .with_runs(
            "topic",
            vec![vec![
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
            ]],
        );

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

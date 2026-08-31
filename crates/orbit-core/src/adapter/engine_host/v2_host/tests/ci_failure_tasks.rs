//! `file_ci_failure_tasks`: clustering, dedupe, and the endings that must stay
//! distinct.
//!
//! Every test drives the action through `run_deterministic`, which is the only
//! way a job step reaches it.

use orbit_engine::RuntimeHost;
use orbit_tools::ToolContext;
use orbit_types::task::TaskStatus;
use serde_json::{Value, json};
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::adapter::engine_host::v2_host::test_support::runtime_with_workspace_layout;
use crate::application::task::TaskUpdateParams;

const HEAD: &str = "1111111111111111111111111111111111111111";
const CHECKOUT: &str = "3333333333333333333333333333333333333333";
const NEXT_HEAD: &str = "4444444444444444444444444444444444444444";

fn file(runtime: &OrbitRuntime, input: Value) -> Value {
    runtime
        .run_deterministic(
            "file_ci_failure_tasks",
            &json!({}),
            &input,
            ToolContext::default(),
        )
        .expect("file ci failure tasks")
}

/// One current failure, shaped exactly as `collect_ci_evidence` emits it.
fn failure(run_id: u64, workflow: &str, job: &str, step: &str, log: &str, checkout: &str) -> Value {
    json!({
        "run_id": run_id,
        "workflow": workflow,
        "title": format!("{workflow} on {HEAD}"),
        "status": "completed",
        "conclusion": "failure",
        "event": "push",
        "url": format!("https://github.com/acme/orbit/actions/runs/{run_id}"),
        "created_at": "2026-08-30T01:00:00Z",
        "head_branch": "agent-main",
        "ref_kind": "integration",
        "pr_number": Value::Null,
        "pr_url": Value::Null,
        "event_reported_head_sha": HEAD,
        "current_ref_head_sha": HEAD,
        "actual_checkout_shas": [checkout],
        "checkout_evidence": [format!("HEAD is now at {checkout}")],
        "checkout_evidence_scope": "all",
        "investigated": true,
        "log_excerpt": log,
        "log_truncated": false,
        "failed_jobs": [{
            "job_id": 900 + run_id,
            "name": job,
            "conclusion": "failure",
            "url": format!("https://github.com/acme/orbit/actions/runs/{run_id}/job/{}", 900 + run_id),
            "failed_steps": [{"name": step, "conclusion": "failure"}],
        }],
    })
}

fn snapshot(current: Vec<Value>) -> Value {
    json!({
        "schema_version": 1,
        "collected": true,
        "outcome_hint": if current.is_empty() { "no_current_failure" } else { "current_failures" },
        "capability": {
            "available": true,
            "authenticated": true,
            "detail": "GitHub CLI is authenticated on this host",
        },
        "repository": {"name": "orbit", "full_name": "acme/orbit", "default_branch": "main"},
        "heads": [{"kind": "integration", "branch": "agent-main", "current_head_sha": HEAD}],
        "current_failures": current,
        "stale_or_superseded": [],
        "in_flight": [],
        "query_errors": [],
        "truncation": {"runs_listed": 4, "current_failures_discovered": 1, "notes": []},
        "collected_at": "2026-08-30T02:00:00Z",
    })
}

fn filed_task_ids(output: &Value) -> Vec<String> {
    output["filed"]
        .as_array()
        .expect("filed array")
        .iter()
        .map(|entry| entry["task_id"].as_str().expect("task id").to_string())
        .collect()
}

/// A workspace whose crew roster has no `system` entry: an explicit `[crews]`
/// table naming only `sol`, with `workflow.system_crew` pointed at a name that
/// resolves to nothing so the usual `system`-aliasing fallback does not kick
/// in either.
fn runtime_without_system_crew() -> (tempfile::TempDir, OrbitRuntime) {
    let root = tempdir().expect("create tempdir");
    let global = root.path().join("home/.orbit");
    let workspace = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global).expect("global orbit dir");
    std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
    std::fs::write(
        workspace.join("config.toml"),
        r#"[workflow]
default_crew = "sol"
system_crew = "not-a-real-crew"

[crews.sol]
provider = "codex"
model = "gpt-5.6-sol"
"#,
    )
    .expect("write crew config with no system entry");
    let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
    (root, runtime)
}

#[test]
fn a_snapshot_that_could_not_look_reports_capability_unavailable_and_files_nothing() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    let output = file(
        &runtime,
        json!({"ci_evidence": {
            "schema_version": 1,
            "collected": false,
            "outcome_hint": "capability_unavailable",
            "capability": {
                "available": true,
                "authenticated": false,
                "detail": "GitHub CLI is present but holds no usable credentials on this host",
            },
            "collected_at": "2026-08-30T02:00:00Z",
        }}),
    );

    assert_eq!(output["outcome"], json!("capability_unavailable"));
    assert_eq!(output["filed_count"], json!(0));
    assert_eq!(output["filed"], json!([]));
    assert_eq!(output["clusters"], json!(0));
    // The distinction that matters: this must never read as a clean CI result.
    assert_ne!(output["outcome"], json!("no_current_failure"));
    assert_eq!(output["capability"]["authenticated"], json!(false));
    assert!(
        runtime
            .list_tasks_by_tags(&["ci-failure-sweep".to_string()])
            .expect("list tasks")
            .is_empty()
    );
}

#[test]
fn no_current_failure_is_a_clean_no_op_and_not_a_capability_problem() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    let output = file(&runtime, json!({"ci_evidence": snapshot(Vec::new())}));

    assert_eq!(output["outcome"], json!("no_current_failure"));
    assert_eq!(output["filed_count"], json!(0));
    assert_ne!(output["outcome"], json!("capability_unavailable"));
    assert_eq!(output["capability"]["authenticated"], json!(true));
    assert!(
        runtime
            .list_tasks_by_tags(&["ci-failure-sweep".to_string()])
            .expect("list tasks")
            .is_empty()
    );
}

#[test]
fn one_regression_across_push_and_pull_request_runs_becomes_one_task() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = "ci\tbuild\t2026-08-30T01:00:00Z error: expected 3 arguments, found 2\n";
    // Same workflow, job, step, error, and tested commit — reported once as a
    // push run and once as a pull-request run.
    let mut pull_request = failure(11, "ci", "build", "cargo build", log, CHECKOUT);
    pull_request["event"] = json!("pull_request");
    pull_request["ref_kind"] = json!("pull_request");
    pull_request["pr_number"] = json!(42);

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![
            failure(10, "ci", "build", "cargo build", log, CHECKOUT),
            pull_request,
            // A genuinely different root cause in the same snapshot.
            failure(12, "lint", "clippy", "cargo clippy", "lint\tclippy\t2026-08-30T01:00:00Z error: unused variable `x`\n", CHECKOUT),
        ])}),
    );

    assert_eq!(output["clusters"], json!(2));
    assert_eq!(output["filed_count"], json!(2));
    let filed = output["filed"].as_array().expect("filed");
    assert_eq!(filed[0]["run_urls"].as_array().expect("urls").len(), 2);
    assert_eq!(filed[1]["run_urls"].as_array().expect("urls").len(), 1);
    assert_ne!(filed[0]["failure_key"], filed[1]["failure_key"]);
}

#[test]
fn a_filed_task_is_an_ordinary_backlog_bug_carrying_usable_evidence() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = "ci\ttest\t2026-08-30T01:00:00Z assertion failed: left == right\n";

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10,
            "ci",
            "test (ubuntu)",
            "cargo test",
            log,
            CHECKOUT,
        )])}),
    );

    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("one filed task");
    let task = runtime.get_task(&task_id).expect("read filed task");

    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.task_type, orbit_types::task::TaskType::Bug);
    // No `github.*` requirement: the evidence is in the description, so the
    // task ships on the ordinary agent baseline.
    assert!(task.required_tools.is_empty());
    assert!(task.tags.contains(&"ci-failure-sweep".to_string()));
    assert!(
        task.tags
            .iter()
            .any(|tag| tag.starts_with("ci-failure:") && tag.len() > "ci-failure:".len())
    );
    assert!(!task.acceptance_criteria.is_empty());

    let description = &task.description;
    for expected in [
        "ci",
        "test (ubuntu)",
        "cargo test",
        "assertion failed: left == right",
        "https://github.com/acme/orbit/actions/runs/10",
        CHECKOUT,
        HEAD,
    ] {
        assert!(
            description.contains(expected),
            "filed description must carry `{expected}`; got:\n{description}"
        );
    }
    // The three commits stay separately labelled rather than collapsing.
    assert!(description.contains("event-reported head SHA"));
    assert!(description.contains("current head of that ref"));
    assert!(description.contains("commit actually checked out"));
    // Bounds are reported so "no more failures" is never read as "we stopped
    // looking".
    assert!(description.contains("Collection bounds"));
}

#[test]
fn a_second_sweep_over_a_still_red_run_does_not_file_a_second_task() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = "ci\tbuild\t2026-08-30T01:00:00Z error: expected 3 arguments, found 2\n";
    let evidence = snapshot(vec![failure(
        10,
        "ci",
        "build",
        "cargo build",
        log,
        CHECKOUT,
    )]);

    let first = file(&runtime, json!({"ci_evidence": evidence}));
    let task_id = filed_task_ids(&first)
        .first()
        .cloned()
        .expect("first sweep files one task");

    // An hour later the same run is still red, and the branch has also moved
    // on with the failure unfixed — so the newer run reports a different tested
    // commit. Dedupe is keyed on the root cause, not the commit, precisely so
    // this does not file again.
    let later = snapshot(vec![
        failure(10, "ci", "build", "cargo build", log, CHECKOUT),
        failure(13, "ci", "build", "cargo build", log, NEXT_HEAD),
    ]);
    let second = file(&runtime, json!({"ci_evidence": later}));

    assert_eq!(second["outcome"], json!("current_failures"));
    assert_eq!(second["filed_count"], json!(0));
    let skipped = second["skipped_existing"].as_array().expect("skipped");
    assert!(!skipped.is_empty());
    assert!(
        skipped
            .iter()
            .all(|entry| entry["task_id"] == json!(task_id.clone())),
        "dedupe must name the open task that already covers the cause: {skipped:?}"
    );
    assert_eq!(
        runtime
            .list_tasks_by_tags(&["ci-failure-sweep".to_string()])
            .expect("list tasks")
            .len(),
        1
    );
}

#[test]
fn a_closed_task_does_not_suppress_a_recurrence() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = "ci\tbuild\t2026-08-30T01:00:00Z error: expected 3 arguments, found 2\n";
    let evidence = snapshot(vec![failure(
        10,
        "ci",
        "build",
        "cargo build",
        log,
        CHECKOUT,
    )]);

    let first = file(&runtime, json!({"ci_evidence": evidence.clone()}));
    let task_id = filed_task_ids(&first)
        .first()
        .cloned()
        .expect("first sweep files one task");
    runtime
        .update_task(
            &task_id,
            TaskUpdateParams {
                status: Some(TaskStatus::Done),
                ..TaskUpdateParams::default()
            },
        )
        .expect("close the first task");

    let second = file(&runtime, json!({"ci_evidence": evidence}));

    assert_eq!(second["filed_count"], json!(1));
    assert_ne!(filed_task_ids(&second).first(), Some(&task_id));
}

#[test]
fn a_listed_but_uninvestigated_failure_is_not_filed_as_an_evidence_free_task() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let mut uninvestigated = failure(10, "ci", "build", "cargo build", "", CHECKOUT);
    uninvestigated["investigated"] = json!(false);
    uninvestigated["failed_jobs"] = json!([]);
    uninvestigated["log_excerpt"] = json!("");

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![uninvestigated])}),
    );

    assert_eq!(output["clusters"], json!(0));
    assert_eq!(output["outcome"], json!("no_current_failure"));
}

#[test]
fn a_filed_task_title_carries_the_sweep_prefix_and_the_system_crew() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = "ci\ttest\t2026-08-30T01:00:00Z assertion failed: left == right\n";

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10,
            "ci",
            "test (ubuntu)",
            "cargo test",
            log,
            CHECKOUT,
        )])}),
    );

    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("one filed task");
    let task = runtime.get_task(&task_id).expect("read filed task");

    assert!(
        task.title
            .starts_with("[ci-failure-sweep] Fix red CI: ci / test (ubuntu) / cargo test"),
        "title must carry the sweep prefix followed by the existing rendering: {}",
        task.title
    );
    assert_eq!(task.crew.as_deref(), Some("system"));
}

#[test]
fn an_over_long_cluster_yields_an_intact_prefix_within_the_existing_bound() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let long_workflow = "w".repeat(300);
    let log = "ci\tbuild\t2026-08-30T01:00:00Z error: boom\n";

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10,
            &long_workflow,
            "build",
            "cargo build",
            log,
            CHECKOUT,
        )])}),
    );

    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("one filed task");
    let task = runtime.get_task(&task_id).expect("read filed task");

    assert!(
        task.title.starts_with("[ci-failure-sweep] Fix red CI: "),
        "prefix must survive truncation intact: {}",
        task.title
    );
    assert!(
        task.title.chars().count() <= 121,
        "title must still respect the existing length bound: {} chars",
        task.title.chars().count()
    );
}

#[test]
fn filing_still_succeeds_in_a_workspace_with_no_system_crew_entry() {
    let (_root, runtime) = runtime_without_system_crew();
    assert!(
        runtime.validate_crew_name(Some("system")).is_err(),
        "fixture must genuinely lack a resolvable system crew"
    );
    let log = "ci\tbuild\t2026-08-30T01:00:00Z error: expected 3 arguments, found 2\n";

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10,
            "ci",
            "build",
            "cargo build",
            log,
            CHECKOUT,
        )])}),
    );

    assert_eq!(output["outcome"], json!("current_failures"));
    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("filing still succeeds without a system crew");
    let task = runtime.get_task(&task_id).expect("read filed task");
    assert_eq!(task.crew, None);
}

#[test]
fn the_filing_cap_reports_what_it_left_unfiled() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let failures: Vec<Value> = (0..3)
        .map(|index| {
            failure(
                10 + index,
                &format!("workflow-{index}"),
                "build",
                "cargo build",
                &format!("w\tbuild\t2026-08-30T01:00:00Z error: cause number {index} here\n"),
                CHECKOUT,
            )
        })
        .collect();

    let output = file(
        &runtime,
        json!({"ci_evidence": snapshot(failures), "max_tasks": 1}),
    );

    assert_eq!(output["clusters"], json!(3));
    assert_eq!(output["filed_count"], json!(1));
    assert_eq!(
        output["skipped_over_cap"]
            .as_array()
            .expect("over-cap array")
            .len(),
        2,
        "a cap must be reported, never a silent truncation"
    );
}

fn signature_line(description: &str) -> String {
    description
        .lines()
        .find(|line| line.contains("Normalized error signature"))
        .expect("signature line")
        .to_string()
}

fn excerpt_block(description: &str) -> String {
    let start = description
        .find("## Failed-step log excerpt")
        .expect("excerpt heading");
    let rest = &description[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end].to_string()
}

/// Realistic GitHub failed-step log: `##[group]Run …`, a large `env:` dump,
/// then the trailing compiler diagnostic. Each env line is ~90 bytes so 50
/// lines already sit past the 4,000-byte description budget.
fn realistic_github_step_log(command: &str, env_lines: usize, trailing: &str) -> String {
    let prefix = |msg: &str| format!("build\tRun go build\t2026-08-30T01:00:00Z {msg}");
    let mut out = String::new();
    out.push_str(&prefix(&format!("##[group]Run {command}")));
    out.push('\n');
    out.push_str(&prefix("env:"));
    out.push('\n');
    for index in 0..env_lines {
        out.push_str(&prefix(&format!("  VAR_{index}: {}", "x".repeat(60))));
        out.push('\n');
    }
    out.push_str(&prefix("##[endgroup]"));
    out.push('\n');
    out.push_str(trailing);
    if !trailing.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn dani_10111_style_log(commit_subject: &str, error: Option<&str>) -> String {
    let prefix = |msg: &str| format!("Vulnerability scan\tCheckout\t2026-08-30T01:00:00Z {msg}\n");
    let mut out = String::new();
    out.push_str(&prefix("##[group]Run actions/checkout@v4"));
    out.push_str(&prefix("with:"));
    out.push_str(&prefix("  repository: acme/monodev"));
    out.push_str(&prefix("  token: ***"));
    out.push_str(&prefix("env:"));
    out.push_str(&prefix("  GITHUB_TOKEN: ***"));
    out.push_str(&prefix("##[endgroup]"));
    out.push_str(&prefix("Syncing repository: acme/monodev"));
    out.push_str(&prefix(&format!("HEAD is now at e5c1dc9 {commit_subject}")));
    if let Some(error) = error {
        out.push_str(&prefix(error));
    }
    out
}

fn filed_description(runtime: &OrbitRuntime, log: &str) -> (Value, String) {
    let output = file(
        runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10,
            "ci",
            "build",
            "cargo build",
            log,
            CHECKOUT,
        )])}),
    );
    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("one filed task");
    let task = runtime.get_task(&task_id).expect("read filed task");
    (output, task.description)
}

#[test]
fn excerpt_keeps_the_run_command_and_trailing_error_not_the_env_dump() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let trailing = concat!(
        "build\tRun go build\t2026-08-30T01:00:00Z ##[command]go build ./...\n",
        "build\tRun go build\t2026-08-30T01:00:00Z ./main.go:10:2: undefined: Foo\n",
        "build\tRun go build\t2026-08-30T01:00:00Z ##[error]Process completed with exit code 1.\n",
    );
    let log = realistic_github_step_log("go build ./...", 80, trailing);
    let error_at = log
        .find("undefined: Foo")
        .expect("fixture must contain the trailing error");
    assert!(
        error_at > 4_000,
        "fixture must place the error past the 4,000-byte description budget, at {error_at}"
    );

    let (_output, description) = filed_description(&runtime, &log);
    let excerpt = excerpt_block(&description);
    assert!(
        excerpt.contains("##[group]Run go build ./..."),
        "excerpt must keep the runner command:\n{excerpt}"
    );
    assert!(
        excerpt.contains("undefined: Foo"),
        "excerpt must carry the trailing error, not a head window:\n{excerpt}"
    );
    assert!(
        excerpt.contains("##[error]Process completed with exit code 1."),
        "excerpt must carry the annotated error:\n{excerpt}"
    );
    assert!(
        !excerpt.contains("VAR_0:"),
        "excerpt must drop the env dump:\n{excerpt}"
    );
}

#[test]
fn excerpt_without_an_error_anchor_says_so_and_still_shows_the_command() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = realistic_github_step_log("go build ./...", 80, "");

    let (_output, description) = filed_description(&runtime, &log);
    let excerpt = excerpt_block(&description);
    assert!(
        excerpt.contains("##[group]Run go build ./..."),
        "command line must still be shown:\n{excerpt}"
    );
    assert!(
        excerpt.contains("No error anchor was present in the retained excerpt"),
        "missing anchor must be stated, not implied by dumping env:\n{excerpt}"
    );
    assert!(
        !excerpt.contains("VAR_0:"),
        "env dump must not be presented as evidence:\n{excerpt}"
    );
}

#[test]
fn error_signature_prefers_an_annotated_error_over_a_checkout_commit_message() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = dani_10111_style_log(
        "chore: add ci failure sweep routine",
        Some("##[error]GO-2024-2611: yaml: vulnerable dependency"),
    );

    let (_output, description) = filed_description(&runtime, &log);
    let signature = signature_line(&description);
    assert!(
        !signature.to_ascii_lowercase().contains("head is now at"),
        "checkout bookkeeping must not become the signature: {signature}"
    );
    assert!(
        signature.contains("yaml") && signature.contains("vulnerable"),
        "signature must come from the ##[error] line: {signature}"
    );
}

#[test]
fn generic_runner_trailer_does_not_collapse_distinct_unannotated_diagnostics() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let trailer =
        "build\tRun go build\t2026-08-30T01:00:00Z ##[error]Process completed with exit code 1.\n";
    let foo_log = realistic_github_step_log(
        "go build ./...",
        2,
        &format!(
            "build\tRun go build\t2026-08-30T01:00:00Z ./main.go:10:2: undefined: Foo\n{trailer}"
        ),
    );
    let bar_log = realistic_github_step_log(
        "go build ./...",
        2,
        &format!(
            "build\tRun go build\t2026-08-30T01:00:00Z ./main.go:14:2: undefined: Bar\n{trailer}"
        ),
    );

    let first = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![
            failure(10, "ci", "build", "go build", &foo_log, CHECKOUT),
            failure(11, "ci", "build", "go build", &bar_log, CHECKOUT),
        ])}),
    );

    assert_eq!(first["filed_count"], json!(2));
    let filed = first["filed"].as_array().expect("filed");
    assert_ne!(filed[0]["failure_key"], filed[1]["failure_key"]);
    for (task_id, diagnostic) in filed_task_ids(&first).iter().zip(["foo", "bar"]) {
        let description = runtime
            .get_task(task_id)
            .expect("read filed task")
            .description;
        let signature = signature_line(&description).to_ascii_lowercase();
        assert!(
            signature.contains(diagnostic),
            "specific diagnostic must be the signature: {signature}"
        );
        assert!(
            !signature.contains("process completed"),
            "generic runner trailer must not be the signature: {signature}"
        );
    }

    let repeated = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            12, "ci", "build", "go build", &foo_log, NEXT_HEAD,
        )])}),
    );
    assert_eq!(repeated["filed_count"], json!(0));
    assert_eq!(
        repeated["skipped_existing"][0]["failure_key"], filed[0]["failure_key"],
        "the same diagnostic must retain its failure key across commits"
    );
}

#[test]
fn checkout_commit_message_containing_failure_is_not_the_signature() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let log = dani_10111_style_log("chore: add ci failure sweep routine", None);

    let (_output, description) = filed_description(&runtime, &log);
    let signature = signature_line(&description);
    assert!(
        signature.contains("step-name fallback"),
        "bookkeeping-only excerpt must label the step-name fallback: {signature}"
    );
    assert!(
        !signature.to_ascii_lowercase().contains("head is now at"),
        "HEAD is now at <hex> chore: … failure … must not be chosen: {signature}"
    );
}

#[test]
fn same_failure_under_a_different_commit_message_reuses_the_failure_key() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let first_log = dani_10111_style_log(
        "chore: add ci failure sweep routine",
        Some("##[error]GO-2024-2611: yaml: vulnerable dependency"),
    );
    let second_log = dani_10111_style_log(
        "chore: mention failure in a later commit",
        Some("##[error]GO-2024-2611: yaml: vulnerable dependency"),
    );

    let first = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            10, "ci", "build", "cargo build", &first_log, CHECKOUT,
        )])}),
    );
    let task_id = filed_task_ids(&first)
        .first()
        .cloned()
        .expect("first sweep files one task");
    let first_key = first["filed"][0]["failure_key"].clone();

    let second = file(
        &runtime,
        json!({"ci_evidence": snapshot(vec![failure(
            11, "ci", "build", "cargo build", &second_log, NEXT_HEAD,
        )])}),
    );

    assert_eq!(second["filed_count"], json!(0));
    let skipped = second["skipped_existing"].as_array().expect("skipped");
    assert!(
        skipped.iter().any(|entry| {
            entry["task_id"] == json!(task_id.clone()) && entry["failure_key"] == first_key
        }),
        "skip_if_open must suppress the second filing under a different commit message: {skipped:?}"
    );
}

#[test]
fn empty_excerpt_surfaces_the_matching_log_query_error() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let mut missing_log = failure(10, "ci", "build", "Run CI guardrails", "", CHECKOUT);
    missing_log["log_excerpt"] = json!("");
    let mut evidence = snapshot(vec![missing_log]);
    evidence["query_errors"] = json!([
        {
            "query": "run_logs",
            "run_id": "10",
            "error": "HTTP 404: Not Found — logs for this run are no longer available"
        },
        {
            "query": "run_list",
            "branch": "other",
            "error": "unrelated list failure"
        }
    ]);

    let output = file(&runtime, json!({"ci_evidence": evidence}));
    let task_id = filed_task_ids(&output)
        .first()
        .cloned()
        .expect("one filed task");
    let description = runtime.get_task(&task_id).expect("read").description;
    let excerpt = excerpt_block(&description);
    assert!(
        excerpt.contains("No log excerpt was captured"),
        "empty excerpt must still be stated:\n{excerpt}"
    );
    assert!(
        excerpt.contains("run_logs")
            && excerpt.contains("10")
            && excerpt.contains("logs for this run are no longer available"),
        "relevant query_errors entry must sit next to the empty excerpt:\n{excerpt}"
    );
    assert!(
        !excerpt.contains("unrelated list failure"),
        "unrelated query errors must not be inlined into this cluster's excerpt:\n{excerpt}"
    );
    let signature = signature_line(&description);
    assert!(
        signature.contains("step-name fallback"),
        "step-name signature must be labeled as a fallback: {signature}"
    );
}

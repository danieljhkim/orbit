use orbit_engine::RuntimeHost;
use orbit_tools::ToolContext;
use orbit_types::task::{TaskPriority, TaskStatus};
use serde_json::{Value, json};
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::adapter::engine_host::v2_host::test_support::runtime_with_workspace_layout;

fn alert(number: u64, severity: &str, range: &str, ghsa: &str) -> Value {
    json!({
        "number": number, "state": "open", "ecosystem": "cargo", "package": "time",
        "manifest_path": "Cargo.lock", "scope": "runtime", "severity": severity,
        "ghsa_id": ghsa, "cve_id": format!("CVE-2026-{number}"),
        "summary": format!("vulnerability {number}"), "vulnerable_version_range": range,
        "first_patched_version": "1.2.3", "html_url": format!("https://github.test/alerts/{number}")
    })
}

fn snapshot(alerts: Vec<Value>, pull_requests: Vec<Value>) -> Value {
    json!({
        "schema_version": 1, "collected": true,
        "outcome_hint": if alerts.is_empty() { "no_open_alerts" } else { "open_alerts" },
        "capability": {"available": true, "authenticated": true, "detail": "authenticated"},
        "repository": {"full_name": "acme/orbit"},
        "open_alerts": alerts,
        "open_dependabot_pull_requests": pull_requests,
        "query_errors": [],
        "truncation": {"alerts_limit": 100, "alerts_at_cap": false},
        "collected_at": "2026-08-31T00:00:00Z"
    })
}

fn file(runtime: &OrbitRuntime, snapshot: Value, extra: Value) -> Value {
    let mut input = json!({"dependabot_snapshot": snapshot});
    if let (Some(target), Some(source)) = (input.as_object_mut(), extra.as_object()) {
        target.extend(source.clone());
    }
    runtime
        .run_deterministic(
            "file_dependabot_alert_tasks",
            &json!({}),
            &input,
            ToolContext::default(),
        )
        .expect("file Dependabot tasks")
}

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
    .expect("write crew config");
    let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
    (root, runtime)
}

#[test]
fn several_alerts_for_one_dependency_file_exactly_one_evidence_complete_task() {
    let (_root, runtime, _repo) = runtime_with_workspace_layout();
    let output = file(
        &runtime,
        snapshot(
            vec![
                alert(1, "high", "< 1.0.0", "GHSA-one"),
                alert(2, "critical", "< 1.2.3", "GHSA-two"),
            ],
            Vec::new(),
        ),
        json!({}),
    );
    assert_eq!(output["filed_count"], json!(1));
    assert_eq!(output["clusters"], json!(1));
    let task_id = output["filed"][0]["task_id"].as_str().expect("task id");
    let task = runtime.get_task(task_id).expect("filed task");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.priority, TaskPriority::Critical);
    assert!(task.required_tools.is_empty());
    assert!(task.title.starts_with("[dependabot-sweep] "));
    for expected in [
        "time",
        "Cargo.lock",
        "1.2.3",
        "GHSA-one",
        "CVE-2026-1",
        "GHSA-two",
        "https://github.test/alerts/2",
    ] {
        assert!(task.description.contains(expected), "missing {expected}");
    }
}

#[test]
fn dedupe_key_survives_changed_alert_number_severity_and_range() {
    let (_root, runtime, _repo) = runtime_with_workspace_layout();
    let first = file(
        &runtime,
        snapshot(vec![alert(1, "high", "< 1.0.0", "GHSA-old")], Vec::new()),
        json!({}),
    );
    assert_eq!(first["filed_count"], json!(1));
    let second = file(
        &runtime,
        snapshot(
            vec![alert(99, "critical", "< 9.9.9", "GHSA-new")],
            Vec::new(),
        ),
        json!({}),
    );
    assert_eq!(second["filed_count"], json!(0));
    assert_eq!(
        second["skipped_existing"]
            .as_array()
            .expect("skipped")
            .len(),
        1
    );
    assert_eq!(
        first["filed"][0]["key"],
        second["skipped_existing"][0]["key"]
    );
}

#[test]
fn severity_floor_is_reported_and_dependabot_pr_skip_is_switchable() {
    let (_root, runtime, _repo) = runtime_with_workspace_layout();
    let low = file(
        &runtime,
        snapshot(
            vec![alert(1, "moderate", "< 1.0.0", "GHSA-low")],
            Vec::new(),
        ),
        json!({}),
    );
    assert_eq!(low["filed_count"], json!(0));
    assert_eq!(
        low["excluded_below_min_severity"]
            .as_array()
            .expect("excluded")
            .len(),
        1
    );

    let pr = json!({"number": 4, "title": "Bump time from 1.0.0 to 1.2.3", "body": "Updates time", "url": "https://github.test/pr/4", "author": "app/dependabot"});
    let skipped = file(
        &runtime,
        snapshot(
            vec![alert(2, "high", "< 1.2.3", "GHSA-high")],
            vec![pr.clone()],
        ),
        json!({}),
    );
    assert_eq!(skipped["filed_count"], json!(0));
    assert_eq!(
        skipped["skipped_dependabot_pr"]
            .as_array()
            .expect("PR skips")
            .len(),
        1
    );

    let filed = file(
        &runtime,
        snapshot(vec![alert(2, "high", "< 1.2.3", "GHSA-high")], vec![pr]),
        json!({"skip_when_dependabot_pr_open": false}),
    );
    assert_eq!(filed["filed_count"], json!(1));
    assert_eq!(filed["skipped_dependabot_pr"], json!([]));
}

#[test]
fn filing_succeeds_without_a_system_crew() {
    let (_root, runtime) = runtime_without_system_crew();
    let output = file(
        &runtime,
        snapshot(
            vec![alert(7, "high", "< 1.2.3", "GHSA-no-crew")],
            Vec::new(),
        ),
        json!({}),
    );
    assert_eq!(output["filed_count"], json!(1));
    let task_id = output["filed"][0]["task_id"].as_str().expect("task id");
    assert_eq!(runtime.get_task(task_id).expect("task").crew, None);
}

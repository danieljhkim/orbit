//! Shared test helpers and child module declarations for run command tests.

use crate::OrbitRuntime;

mod actions;
mod owner;
mod reconcile;

use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState};
use tempfile::tempdir;

pub(crate) fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime)
}

pub(crate) fn insert_pending_run(runtime: &OrbitRuntime, job_id: &str) -> JobRun {
    runtime
        .stores()
        .jobs()
        .insert_run(
            job_id,
            1,
            Utc::now() - chrono::Duration::seconds(5),
            None,
            None,
        )
        .expect("insert run")
}

pub(crate) fn strip_run_timing(runtime: &OrbitRuntime, run: &JobRun) {
    let path = runtime
        .data_root()
        .join("state")
        .join("job-runs")
        .join(&run.job_id)
        .join(&run.run_id)
        .join("jrun.yaml");
    let raw = std::fs::read_to_string(&path).expect("read run yaml");
    let edited = raw
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("finished_at:") {
                "  finished_at: null".to_string()
            } else if line.trim_start().starts_with("duration_ms:") {
                "  duration_ms: null".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{edited}\n")).expect("write run yaml");
}

pub(crate) fn write_run_finished_audit(
    runtime: &OrbitRuntime,
    run_id: &str,
    finished_at: DateTime<Utc>,
) {
    let dir = runtime
        .data_root()
        .join("state")
        .join("audit")
        .join("v2_loop");
    std::fs::create_dir_all(&dir).expect("create audit dir");
    let line = serde_json::json!({
        "event_type": "run.finished",
        "ts": finished_at.to_rfc3339(),
        "outcome": "success",
        "error_message": null,
    });
    std::fs::write(dir.join(format!("{run_id}.jsonl")), format!("{line}\n"))
        .expect("write audit event");
}

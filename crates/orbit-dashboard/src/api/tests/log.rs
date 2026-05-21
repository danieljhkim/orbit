use std::io::Write;

use serde_json::json;
use tempfile::tempdir;

use super::super::LogQuery;
use super::super::log::{
    LOG_MAX_LIMIT, format_sse_frame, read_appended_log_events, read_log_snapshot_from_path,
};
use super::test_support::write_lines;
use crate::log_format::Filters as LogFilters;

#[test]
fn log_snapshot_filters_target_level_and_since() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("orbit.jsonl");
    write_lines(
        &path,
        &[
            json!({
                "timestamp": "2026-04-27T01:00:01Z",
                "level": "INFO",
                "target": "orbit.policy.deny",
                "fields": {"tool": "fs.read", "path": "/tmp/a"}
            })
            .to_string(),
            json!({
                "timestamp": "2026-04-27T01:00:03Z",
                "level": "WARN",
                "target": "orbit.policy.deny",
                "fields": {"tool": "fs.write", "path": "/etc/passwd"}
            })
            .to_string(),
            json!({
                "timestamp": "2026-04-27T01:00:04Z",
                "level": "ERROR",
                "target": "orbit.job.step_finished",
                "fields": {"step_id": "build", "outcome": "failed", "success": false}
            })
            .to_string(),
        ],
    );

    let events = read_log_snapshot_from_path(
        &path,
        &LogQuery {
            limit: Some(10),
            target: Some("orbit.policy".to_string()),
            level: Some("warn".to_string()),
            since: Some("2026-04-27T01:00:02Z".to_string()),
        },
    )
    .expect("snapshot");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, "policy");
    assert_eq!(events[0].code, "DENY");
    assert_eq!(events[0].level, "warn");
    assert!(events[0].message_html.contains("<b>path</b>="));
}

#[test]
fn log_snapshot_rejects_limit_above_max() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("orbit.jsonl");
    write_lines(&path, &[]);

    let err = read_log_snapshot_from_path(
        &path,
        &LogQuery {
            limit: Some(LOG_MAX_LIMIT + 1),
            ..LogQuery::default()
        },
    )
    .expect_err("limit should be rejected");

    assert!(err.to_string().contains("limit must be <= 500"));
}

#[test]
fn log_stream_framing_emits_one_data_frame_per_appended_line() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("orbit.jsonl");
    write_lines(&path, &[]);
    let mut offset = std::fs::metadata(&path).expect("metadata").len();
    let mut leftover = String::new();

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-04-27T01:00:05Z",
            "level": "INFO",
            "target": "orbit.job.step_started",
            "fields": {"job_run_id": "run-1", "step_id": "build"}
        })
    )
    .expect("write event");
    file.flush().expect("flush");

    let events =
        read_appended_log_events(&path, &LogFilters::default(), &mut offset, &mut leftover)
            .expect("read appended");
    assert_eq!(events.len(), 1);

    let frame = format_sse_frame(&events[0]).expect("frame");
    assert!(frame.starts_with("data: "));
    assert!(frame.ends_with("\n\n"));
    assert!(frame.contains("\"source\":\"job\""));
    assert!(frame.contains("build"));
}

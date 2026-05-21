use std::ffi::OsString;

use regex::Regex;
use serde_json::Value;
use tempfile::tempdir;

use super::super::*;
use super::{
    BufferMakeWriter, ENV_LOCK, EnvVarGuard, read_jsonl_values,
    with_test_subscriber_allowing_file_failure, with_test_subscriber_at_path,
};

#[test]
fn jsonl_layer_honors_rust_log_filter() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::set("RUST_LOG", OsString::from("orbit_common=debug"));
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("trace", &log_path, io::sink, || {
        tracing::debug!(target: "orbit_common::filter_probe", accepted = true);
        tracing::trace!(target: "orbit_common::filter_probe", rejected = true);
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["level"], "DEBUG");
    assert_eq!(values[0]["fields"]["accepted"], true);
    assert!(values[0]["fields"].get("rejected").is_none());
}

#[test]
fn jsonl_event_contains_required_shape_and_fields() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!(provider = "codex", stream = "stdout", line = "hi");
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    let event = &values[0];
    let timestamp = event["timestamp"].as_str().expect("timestamp string");
    let timestamp_re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}").expect("valid regex");
    assert!(
        timestamp_re.is_match(timestamp),
        "timestamp should be ISO-like, got {timestamp}"
    );
    assert_eq!(event["level"], "INFO");
    assert!(event.get("target").is_some());
    assert_eq!(event["fields"]["provider"], "codex");
    assert_eq!(event["fields"]["stream"], "stdout");
    assert_eq!(event["fields"]["line"], "hi");
}

#[test]
fn jsonl_event_preserves_cli_runner_structured_fields() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!(
            provider = "codex",
            stream = "stderr",
            job_run_id = "jrun-123",
            task_id = "T20260426-2343",
            line = "hello"
        );
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    let fields = &values[0]["fields"];
    assert_eq!(fields["provider"], "codex");
    assert_eq!(fields["stream"], "stderr");
    assert_eq!(fields["job_run_id"], "jrun-123");
    assert_eq!(fields["task_id"], "T20260426-2343");
    assert_eq!(fields["line"], "hello");
}

#[test]
fn jsonl_file_appends_to_existing_content() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");
    fs::write(&log_path, "sentinel\n").expect("write sentinel");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!(line = "after-sentinel");
    })
    .expect("subscriber should run");

    let content = fs::read_to_string(&log_path).expect("read log");
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "sentinel");
    let appended: Value = serde_json::from_str(lines[1]).expect("appended line is json");
    assert_eq!(appended["fields"]["line"], "after-sentinel");
}

#[test]
fn file_layer_failure_falls_back_to_stderr_layer() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let blocked_parent = dir.path().join("not-a-directory");
    fs::write(&blocked_parent, "file, not dir").expect("write blocking file");
    let log_path = blocked_parent.join("orbit.jsonl");
    let stderr = BufferMakeWriter::default();
    let stderr_buffer = stderr.buffer();

    let warning = with_test_subscriber_allowing_file_failure("info", &log_path, stderr, || {
        tracing::info!(line = "stderr-still-works");
    })
    .expect("file layer should fail");

    assert!(warning.contains("cannot create JSONL tracing log directory"));
    let stderr_text =
        String::from_utf8(stderr_buffer.lock().expect("stderr lock").clone()).expect("stderr utf8");
    assert!(stderr_text.contains("failed to initialize JSONL tracing log"));
    assert!(stderr_text.contains("stderr-still-works"));
}

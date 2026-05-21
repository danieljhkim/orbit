use std::ffi::OsString;

use tempfile::tempdir;

use super::super::*;
use super::{ENV_LOCK, EnvVarGuard, read_jsonl_values, with_test_subscriber_at_path};

#[test]
fn jsonl_redacting_fields_preserves_typed_values_and_redacts_strings() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!(count = 42, ok = true, secret = "Authorization: Bearer abc");
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    let fields = &values[0]["fields"];
    assert_eq!(fields["count"], 42);
    assert_eq!(fields["ok"], true);
    assert!(
        !fields["secret"]
            .as_str()
            .unwrap_or_default()
            .contains("abc")
    );
}

#[test]
fn jsonl_redacting_fields_preserves_sensitive_field_names() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!(password = "plain-public-value");
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["fields"]["password"], "plain-public-value");
}

#[test]
fn jsonl_redacting_fields_redacts_unstructured_message() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        tracing::info!("Bearer abc123 leaked");
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    let message = values[0]["fields"]["message"]
        .as_str()
        .expect("message is string");
    assert!(message.contains("[REDACTED_AUTH]"));
    assert!(!message.contains("abc123"));
}

#[test]
fn jsonl_redacting_fields_redacts_debug_values() {
    struct Payload {
        header: &'static str,
    }

    impl std::fmt::Debug for Payload {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Payload")
                .field("header", &self.header)
                .finish()
        }
    }

    let _env = ENV_LOCK.lock().expect("lock env");
    let _rust_log = EnvVarGuard::remove("RUST_LOG");
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("orbit.jsonl");

    with_test_subscriber_at_path("info", &log_path, io::sink, || {
        let payload = Payload {
            header: "Authorization: Bearer abc456",
        };
        tracing::info!(payload = ?payload);
    })
    .expect("subscriber should run");

    let values = read_jsonl_values(&log_path);
    assert_eq!(values.len(), 1);
    let payload = values[0]["fields"]["payload"]
        .as_str()
        .expect("payload is string");
    assert!(payload.contains("[REDACTED_AUTH]"));
    assert!(!payload.contains("abc456"));
}

#[test]
fn default_pattern_redactor_is_initialized_once() {
    let first = super::super::redaction::default_pattern_redactor();
    let second = super::super::redaction::default_pattern_redactor();

    assert!(std::ptr::eq(first, second));
}

#[test]
fn redact_event_text_still_scrubs_sensitive_text() {
    let _env = ENV_LOCK.lock().expect("lock env");
    let _secret = EnvVarGuard::set("ORBIT_TEST_TOKEN", OsString::from("super-secret-value"));

    let redacted = redact_event_text("token is super-secret-value");

    assert!(!redacted.contains("super-secret-value"));
    assert!(redacted.contains("[REDACTED_ENV]"));
}

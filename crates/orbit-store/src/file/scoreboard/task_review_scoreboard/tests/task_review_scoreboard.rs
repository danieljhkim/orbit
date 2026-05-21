// Migrated from file/scoreboard/task_review_scoreboard.rs per ORB-00231
use std::fs;

use super::super::*;
use serde_json::Value;

#[test]
fn record_task_review_thread_migrates_legacy_message_metric() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("task_review.json"),
        r#"{"task-review-messages":{"gpt-5.4":2}}"#,
    )
    .expect("write legacy scoreboard");

    record_task_review_thread(temp.path(), "gpt-5.4").expect("record thread score");

    let raw =
        fs::read_to_string(temp.path().join("task_review.json")).expect("read migrated scoreboard");
    let scoreboard: Value = serde_json::from_str(&raw).expect("parse migrated scoreboard");
    assert!(scoreboard["task-review-messages"].is_null());
    assert_eq!(scoreboard["task-review-threads"]["gpt-5.4"], Value::from(3));
}

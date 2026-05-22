//! Sibling tests for `run_input.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use serde_json::json;

use super::super::run_input::singular_task_id_from_input;

#[test]
fn singular_task_id_accepts_single_entry_task_ids() {
    let input = json!({ "task_ids": [" ORB-00073 "] });

    assert_eq!(singular_task_id_from_input(&input), Some("ORB-00073"));
}

#[test]
fn singular_task_id_rejects_multi_task_input() {
    let input = json!({ "task_ids": ["ORB-00073", "ORB-00078"] });

    assert_eq!(singular_task_id_from_input(&input), None);
}

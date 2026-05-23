use serde_json::json;

use super::super::run::shape_tool_output;

#[test]
fn list_output_uses_minimal_task_projection() {
    let shaped = shape_tool_output(
        "orbit.task.list",
        &json!({ "status": "backlog" }),
        json!([{
            "id": "T20260422-0001",
            "title": "Backlog task",
            "status": "backlog",
            "priority": "medium",
            "type": "feature",
            "dependencies": [],
            "resolved_dependencies": [],
            "implemented_by": null,
            "created_at": "2026-04-22T00:00:00Z",
            "updated_at": "2026-04-22T00:00:00Z",
            "description": "should be filtered out"
        }]),
        false,
        &[],
    );

    assert_eq!(
        shaped,
        json!([{
            "id": "T20260422-0001",
            "title": "Backlog task",
            "status": "backlog",
            "priority": "medium",
            "type": "feature",
            "dependencies": [],
            "resolved_dependencies": [],
            "implemented_by": null,
            "created_at": "2026-04-22T00:00:00Z",
            "updated_at": "2026-04-22T00:00:00Z"
        }])
    );
}

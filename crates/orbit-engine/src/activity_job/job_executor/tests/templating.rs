#![allow(missing_docs)]

use super::*;

#[test]
fn render_input_supports_legacy_batch_id_from_worktree_output() {
    let mut steps = HashMap::new();
    steps.insert(
        "worktree".to_string(),
        json!({
            "output": {
                "job_run_id": "jrun-template",
                "batch_id": "jrun-template",
            }
        }),
    );
    let tctx = TemplateContext {
        steps,
        ..TemplateContext::default()
    };
    let default_input = json!({
        "batch_id": "{{ steps.worktree.output.batch_id }}",
    });

    let rendered = render_input(Some(&default_input), &Value::Null, &tctx).unwrap();

    assert_eq!(rendered, json!({ "batch_id": "jrun-template" }));
}

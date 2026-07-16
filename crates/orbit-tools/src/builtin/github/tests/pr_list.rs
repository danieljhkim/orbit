use serde_json::json;

use super::super::pr_list::build_exec_request;
use crate::ToolContext;

#[test]
fn branch_lookup_uses_exact_head_filter() {
    let context = ToolContext {
        cwd: Some("/workspace".to_string()),
        ..ToolContext::default()
    };
    let request = build_exec_request(
        &context,
        &json!({
            "head": "orbit/ORB-10240-branch",
            "state": "open",
        }),
    )
    .expect("build list request");

    assert_eq!(
        request.args,
        vec![
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "number,title,headRefName,author",
            "--head",
            "orbit/ORB-10240-branch",
        ]
    );
    assert_eq!(request.current_dir.as_deref(), Some("/workspace"));
}

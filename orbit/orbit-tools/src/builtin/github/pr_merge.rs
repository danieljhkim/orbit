use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_SLOW_MS, check_exec_result};

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let strategy = input
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("squash");

    let strategy_flag = match strategy {
        "squash" => "--squash",
        "merge" => "--merge",
        "rebase" => "--rebase",
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid `strategy`: \"{other}\"; must be squash, merge, or rebase"
            )));
        }
    };

    let delete_branch = input
        .get("delete_branch")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut args = vec![
        "pr".to_string(),
        "merge".to_string(),
        pr,
        strategy_flag.to_string(),
    ];

    if delete_branch {
        args.push("--delete-branch".to_string());
    }

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(args, None, TIMEOUT_SLOW_MS))
}

super::gh_tool! {
    pub struct GithubPrMergeTool;
    name: "github.pr.merge";
    description: "Merge a pull request";
    parameters: [
        super::tool_param("pr", "PR number, URL, or branch name", "string", true),
        super::tool_param(
            "strategy",
            "Merge strategy: squash (default), merge, or rebase",
            "string",
            false,
        ),
        super::tool_param(
            "delete_branch",
            "Delete the head branch after merge (default: true)",
            "bool",
            false,
        ),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr merge")?;
        Ok(json!({
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_exec_request;

    #[test]
    fn build_exec_request_deletes_branch_by_default() {
        let req = build_exec_request(&json!({
            "pr": "76",
            "strategy": "squash",
        }))
        .expect("build request");

        assert!(req.args.iter().any(|arg| arg == "--delete-branch"));
    }

    #[test]
    fn build_exec_request_omits_delete_branch_when_disabled() {
        let req = build_exec_request(&json!({
            "pr": "76",
            "strategy": "squash",
            "delete_branch": false,
        }))
        .expect("build request");

        assert!(!req.args.iter().any(|arg| arg == "--delete-branch"));
    }
}

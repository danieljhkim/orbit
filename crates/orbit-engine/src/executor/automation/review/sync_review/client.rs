use orbit_common::types::OrbitError;
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use serde_json::Value;

const TIMEOUT_MS: u64 = 15_000;

pub(super) trait GhClient {
    fn get_owner_repo(&self, repo_root: &str) -> Result<String, OrbitError>;
    fn get_pr_head_sha(&self, repo_root: &str, pr_number: &str) -> Result<String, OrbitError>;
    fn load_pr_file_patches(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
    ) -> Result<super::patch_match::PrFilePatchMap, OrbitError>;
    #[allow(clippy::too_many_arguments)]
    fn create_inline_review_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        commit_id: &str,
        path: &str,
        line: u64,
        body: &str,
    ) -> Result<u64, OrbitError>;
    fn create_general_comment(
        &self,
        repo_root: &str,
        pr_number: &str,
        body: &str,
    ) -> Result<u64, OrbitError>;
    fn create_reply_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64, OrbitError>;
}

pub(super) struct RealGhClient;

impl GhClient for RealGhClient {
    fn get_owner_repo(&self, repo_root: &str) -> Result<String, OrbitError> {
        get_owner_repo(repo_root)
    }

    fn get_pr_head_sha(&self, repo_root: &str, pr_number: &str) -> Result<String, OrbitError> {
        get_pr_head_sha(repo_root, pr_number)
    }

    fn load_pr_file_patches(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
    ) -> Result<super::patch_match::PrFilePatchMap, OrbitError> {
        load_pr_file_patches(repo_root, owner_repo, pr_number)
    }

    fn create_inline_review_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        commit_id: &str,
        path: &str,
        line: u64,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_inline_review_comment(
            repo_root, owner_repo, pr_number, commit_id, path, line, body,
        )
    }

    fn create_general_comment(
        &self,
        repo_root: &str,
        pr_number: &str,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_general_comment(repo_root, pr_number, body)
    }

    fn create_reply_comment(
        &self,
        repo_root: &str,
        owner_repo: &str,
        pr_number: &str,
        parent_comment_id: u64,
        body: &str,
    ) -> Result<u64, OrbitError> {
        create_reply_comment(repo_root, owner_repo, pr_number, parent_comment_id, body)
    }
}

pub(super) fn get_owner_repo(repo_root: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "repo".to_string(),
                "view".to_string(),
                "--json".to_string(),
                "nameWithOwner".to_string(),
                "-q".to_string(),
                ".nameWithOwner".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to get repo owner/name: {}",
            result.stderr.trim()
        )));
    }

    Ok(result.stdout.trim().to_string())
}

pub(super) fn get_pr_head_sha(repo_root: &str, pr_number: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "headRefOid".to_string(),
                "-q".to_string(),
                ".headRefOid".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to get PR head SHA: {}",
            result.stderr.trim()
        )));
    }

    Ok(result.stdout.trim().to_string())
}

pub(super) fn load_pr_file_patches(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
) -> Result<super::patch_match::PrFilePatchMap, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{owner_repo}/pulls/{pr_number}/files"),
                "--paginate".to_string(),
                "--slurp".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to fetch PR file patches: {}",
            result.stderr.trim()
        )));
    }

    super::patch_match::parse_pr_file_patches(&result.stdout)
}

pub(super) fn create_inline_review_comment(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    commit_id: &str,
    path: &str,
    line: u64,
    body: &str,
) -> Result<u64, OrbitError> {
    let payload = serde_json::json!({
        "body": body,
        "commit_id": commit_id,
        "path": path,
        "line": line,
        "side": "RIGHT",
    });

    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{owner_repo}/pulls/{pr_number}/comments"),
                "--method".to_string(),
                "POST".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Bytes(payload.to_string().into_bytes()),
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to create inline review comment: {}",
            result.stderr.trim()
        )));
    }

    parse_comment_id(&result.stdout)
}

pub(super) fn create_general_comment(
    repo_root: &str,
    pr_number: &str,
    body: &str,
) -> Result<u64, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "comment".to_string(),
                pr_number.to_string(),
                "--body".to_string(),
                body.to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to create PR comment: {}",
            result.stderr.trim()
        )));
    }

    let output = result.stdout.trim();
    if let Some(id_str) = output.rsplit("issuecomment-").next()
        && let Ok(id) = id_str.trim().parse::<u64>()
    {
        return Ok(id);
    }

    Err(OrbitError::Execution(format!(
        "could not parse comment ID from gh pr comment output: {output}"
    )))
}

pub(super) fn create_reply_comment(
    repo_root: &str,
    owner_repo: &str,
    pr_number: &str,
    parent_comment_id: u64,
    body: &str,
) -> Result<u64, OrbitError> {
    let payload = serde_json::json!({ "body": body });

    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!(
                    "repos/{owner_repo}/pulls/{pr_number}/comments/{parent_comment_id}/replies"
                ),
                "--method".to_string(),
                "POST".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Bytes(payload.to_string().into_bytes()),
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to create reply comment: {}",
            result.stderr.trim()
        )));
    }

    parse_comment_id(&result.stdout)
}

fn parse_comment_id(json_output: &str) -> Result<u64, OrbitError> {
    let value: Value = serde_json::from_str(json_output.trim())
        .map_err(|e| OrbitError::Execution(format!("failed to parse GitHub API response: {e}")))?;

    value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| OrbitError::Execution("GitHub API response missing 'id' field".to_string()))
}

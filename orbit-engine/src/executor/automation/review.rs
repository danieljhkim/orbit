use std::path::Path;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::OrbitError;
use serde_json::Value;

pub(super) fn resolve_review_decision(
    repo_root: &Path,
    pr_number: &str,
) -> Result<String, OrbitError> {
    fetch_review_decision_from_gh(repo_root, pr_number)
}

pub(crate) fn normalize_review_decision(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "APPROVED" | "APPROVE" => "APPROVED".to_string(),
        "REQUEST-CHANGES" | "REQUEST_CHANGES" | "CHANGES_REQUESTED" => {
            "CHANGES_REQUESTED".to_string()
        }
        other => other.to_string(),
    }
}

fn fetch_review_decision_from_gh(repo_root: &Path, pr_number: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "reviewDecision".to_string(),
            ],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(15_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "gh pr view failed while fetching reviewDecision for '{pr_number}': {}",
            result.stderr.trim()
        )));
    }

    let payload: Value = serde_json::from_str(&result.stdout).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to parse gh pr view reviewDecision output for '{pr_number}': {error}"
        ))
    })?;
    match payload.get("reviewDecision") {
        // GitHub returns null or "" when no reviews exist or branch protection doesn't require them.
        Some(Value::Null) | None => Ok("NONE".to_string()),
        Some(Value::String(s)) if s.trim().is_empty() => Ok("NONE".to_string()),
        Some(v) => v
            .as_str()
            .map(normalize_review_decision)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                OrbitError::Execution(format!(
                    "gh pr view returned unexpected reviewDecision type for '{pr_number}'"
                ))
            }),
    }
}

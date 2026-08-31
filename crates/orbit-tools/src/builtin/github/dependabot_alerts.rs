use orbit_common::OrbitError;
use orbit_exec::ExecRequest;
use serde_json::{Value, json};

use crate::TIMEOUT_DEFAULT_MS;

const DEFAULT_LIMIT: u64 = 100;
const MAX_LIMIT: u64 = 100;

fn repository_path(input: &Value) -> Result<String, OrbitError> {
    let Some(raw) = input.get("repo").and_then(Value::as_str) else {
        return Ok("{owner}/{repo}".to_string());
    };
    let repo = raw.trim();
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    if parts.next().is_some() || !valid_part(owner) || !valid_part(name) {
        return Err(OrbitError::InvalidInput(format!(
            "invalid `repo`: \"{repo}\"; expected owner/name using ASCII letters, digits, '.', '-', or '_'"
        )));
    }
    Ok(repo.to_string())
}

/// Build one bounded Dependabot-alert API request. This builder is deliberately
/// unregistered: only engine-private host automation calls it.
pub fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let endpoint = format!("repos/{}/dependabot/alerts", repository_path(input)?);
    let limit = super::bounded_limit(input, "limit", DEFAULT_LIMIT, MAX_LIMIT)?;
    let args = vec![
        "api".to_string(),
        "--method".to_string(),
        "GET".to_string(),
        endpoint,
        "-f".to_string(),
        "state=open".to_string(),
        "-F".to_string(),
        format!("per_page={limit}"),
    ];
    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

/// Build the bounded query used to discover Dependabot-authored open PRs.
pub fn build_open_pull_requests_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let limit = super::bounded_limit(input, "limit", DEFAULT_LIMIT, MAX_LIMIT)?;
    let mut args = vec![
        "pr".to_string(),
        "list".to_string(),
        "--state".to_string(),
        "open".to_string(),
        "--author".to_string(),
        "app/dependabot".to_string(),
    ];
    super::push_repo_flag(&mut args, input)?;
    args.extend([
        "--limit".to_string(),
        limit.to_string(),
        "--json".to_string(),
        "number,title,url,body,author".to_string(),
    ]);
    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

pub fn project_alert(alert: &Value) -> Value {
    json!({
        "number": alert["number"],
        "state": alert["state"],
        "ecosystem": alert["dependency"]["package"]["ecosystem"],
        "package": alert["dependency"]["package"]["name"],
        "manifest_path": alert["dependency"]["manifest_path"],
        "scope": alert["dependency"]["scope"],
        "severity": alert["security_advisory"]["severity"],
        "ghsa_id": alert["security_advisory"]["ghsa_id"],
        "cve_id": alert["security_advisory"]["cve_id"],
        "summary": alert["security_advisory"]["summary"],
        "vulnerable_version_range": alert["security_vulnerability"]["vulnerable_version_range"],
        "first_patched_version": alert["security_vulnerability"]["first_patched_version"]["identifier"],
        "created_at": alert["created_at"],
        "updated_at": alert["updated_at"],
        "dismissed_at": alert["dismissed_at"],
        "fixed_at": alert["fixed_at"],
        "html_url": alert["html_url"],
    })
}

pub fn project_pull_request(pr: &Value) -> Value {
    json!({
        "number": pr["number"],
        "title": pr["title"],
        "url": pr["url"],
        "body": pr["body"],
        "author": pr["author"]["login"],
    })
}

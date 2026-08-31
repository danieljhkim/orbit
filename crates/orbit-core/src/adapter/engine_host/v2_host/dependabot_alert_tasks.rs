//! Turn a host-collected Dependabot snapshot into ordinary backlog tasks.

use std::collections::BTreeMap;

use orbit_common::OrbitError;
use orbit_types::task::{TaskComplexity, TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::OrbitRuntime;
use crate::application::task::TaskAddParams;

const SUPPORTED_SCHEMA_VERSION: u64 = 1;
const TAG: &str = "dependabot-sweep";
const KEY_PREFIX: &str = "dependabot:";
const TITLE_PREFIX: &str = "[dependabot-sweep] ";
const SYSTEM_CREW: &str = "system";
const DEFAULT_MAX_TASKS: u64 = 10;
const MAX_TASKS: u64 = 50;
const KEY_LEN: usize = 16;

pub(crate) fn file_dependabot_alert_tasks(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<Value, OrbitError> {
    let snapshot = input.get("dependabot_snapshot").ok_or_else(|| {
        OrbitError::InvalidInput(
            "file_dependabot_alert_tasks requires the `dependabot_snapshot` produced by collect_dependabot_alerts"
                .to_string(),
        )
    })?;
    if !snapshot.is_object() {
        return Err(OrbitError::InvalidInput(
            "dependabot_snapshot must be an object".to_string(),
        ));
    }
    let version = snapshot
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(SUPPORTED_SCHEMA_VERSION);
    if version > SUPPORTED_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "Dependabot snapshot schema version {version} is newer than supported version {SUPPORTED_SCHEMA_VERSION}"
        )));
    }
    let capability = snapshot.get("capability").cloned().unwrap_or(Value::Null);
    if snapshot.get("collected").and_then(Value::as_bool) != Some(true) {
        return Ok(empty_output(
            "capability_unavailable",
            capability,
            "Dependabot alerts could not be queried; this is not a clean vulnerability result",
        ));
    }

    let floor_name = input
        .get("min_severity")
        .and_then(Value::as_str)
        .unwrap_or("high")
        .trim()
        .to_ascii_lowercase();
    let floor = severity_rank(&floor_name).ok_or_else(|| {
        OrbitError::InvalidInput(
            "input.min_severity must be one of low, moderate, high, or critical".to_string(),
        )
    })?;
    let max_tasks = bounded_u64(input, "max_tasks", DEFAULT_MAX_TASKS, MAX_TASKS)? as usize;
    let skip_pr = input
        .get("skip_when_dependabot_pr_open")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let alerts = snapshot
        .get("open_alerts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut excluded_below_min_severity = Vec::new();
    let mut grouped: BTreeMap<(String, String, String), Vec<Value>> = BTreeMap::new();
    for alert in alerts {
        let severity = field(alert, "severity").to_ascii_lowercase();
        let rank = severity_rank(&severity).unwrap_or(0);
        if rank < floor {
            excluded_below_min_severity.push(json!({
                "number": alert.get("number"),
                "ecosystem": field(alert, "ecosystem"),
                "package": field(alert, "package"),
                "manifest_path": field(alert, "manifest_path"),
                "severity": severity,
            }));
            continue;
        }
        let key = (
            field(alert, "ecosystem"),
            field(alert, "package"),
            field(alert, "manifest_path"),
        );
        if key.0.is_empty() || key.1.is_empty() || key.2.is_empty() {
            continue;
        }
        grouped.entry(key).or_default().push(alert.clone());
    }

    if grouped.is_empty() {
        let mut output = empty_output(
            if alerts.is_empty() {
                "no_open_alerts"
            } else {
                "open_alerts"
            },
            capability,
            "No open Dependabot alert met the configured severity floor",
        );
        output["min_severity"] = json!(floor_name);
        output["excluded_below_min_severity"] = json!(excluded_below_min_severity);
        return Ok(output);
    }

    let pull_requests = snapshot
        .get("open_dependabot_pull_requests")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let system_crew = runtime
        .validate_crew_name(Some(SYSTEM_CREW))
        .is_ok()
        .then(|| SYSTEM_CREW.to_string());
    let mut filed = Vec::new();
    let mut skipped_existing = Vec::new();
    let mut skipped_dependabot_pr = Vec::new();
    let mut skipped_over_cap = Vec::new();

    for ((ecosystem, package, manifest_path), mut cluster_alerts) in grouped {
        cluster_alerts
            .sort_by_key(|alert| alert.get("number").and_then(Value::as_u64).unwrap_or(0));
        let key = digest(&ecosystem, &package, &manifest_path);
        if let Some(task_id) = open_task_for_key(runtime, &key)? {
            skipped_existing.push(json!({
                "key": key, "task_id": task_id, "ecosystem": ecosystem,
                "package": package, "manifest_path": manifest_path,
            }));
            continue;
        }
        if skip_pr
            && let Some(pull_request) = pull_requests
                .iter()
                .find(|pull_request| pull_request_mentions_package(pull_request, &package))
        {
            skipped_dependabot_pr.push(json!({
                "key": key, "ecosystem": ecosystem, "package": package,
                "manifest_path": manifest_path, "pull_request": pull_request,
            }));
            continue;
        }
        if filed.len() >= max_tasks {
            skipped_over_cap.push(json!({
                "key": key, "ecosystem": ecosystem, "package": package,
                "manifest_path": manifest_path,
            }));
            continue;
        }

        let highest = cluster_alerts
            .iter()
            .filter_map(|alert| severity_rank(&field(alert, "severity").to_ascii_lowercase()))
            .max()
            .unwrap_or(floor);
        let task = runtime.add_task(TaskAddParams {
            title: task_title(&package, &manifest_path),
            description: task_description(
                snapshot,
                &ecosystem,
                &package,
                &manifest_path,
                &cluster_alerts,
            ),
            acceptance_criteria: vec![
                format!("Bump `{package}` to a non-vulnerable version that resolves every alert listed in the task description."),
                format!("`{manifest_path}` and the repository lockfile agree on the resolved `{package}` version."),
                "The repository's documented pre-handoff checks pass without weakening security checks or suppressing the advisories.".to_string(),
            ],
            tags: vec![TAG.to_string(), format!("{KEY_PREFIX}{key}"), "security".to_string()],
            required_tools: Vec::new(),
            crew: system_crew.clone(),
            priority: priority_for_rank(highest),
            complexity: TaskComplexity::Unassessed,
            task_type: Some(TaskType::Bug),
            status: Some(TaskStatus::Backlog),
            system_created: true,
            ..TaskAddParams::default()
        })?;
        filed.push(json!({
            "task_id": task.id, "key": key, "ecosystem": ecosystem,
            "package": package, "manifest_path": manifest_path,
            "alert_count": cluster_alerts.len(),
        }));
    }

    Ok(json!({
        "outcome": "open_alerts",
        "capability": capability,
        "clusters": filed.len() + skipped_existing.len() + skipped_dependabot_pr.len() + skipped_over_cap.len(),
        "filed_count": filed.len(),
        "filed": filed,
        "skipped_existing": skipped_existing,
        "skipped_dependabot_pr": skipped_dependabot_pr,
        "skipped_over_cap": skipped_over_cap,
        "excluded_below_min_severity": excluded_below_min_severity,
        "min_severity": floor_name,
        "skip_when_dependabot_pr_open": skip_pr,
        "max_tasks": max_tasks,
    }))
}

fn empty_output(outcome: &str, capability: Value, detail: &str) -> Value {
    json!({
        "outcome": outcome, "capability": capability, "detail": detail,
        "clusters": 0, "filed_count": 0, "filed": [], "skipped_existing": [],
        "skipped_dependabot_pr": [], "skipped_over_cap": [],
        "excluded_below_min_severity": [],
    })
}

fn task_title(package: &str, manifest_path: &str) -> String {
    let budget = 120usize.saturating_sub(TITLE_PREFIX.chars().count());
    let body = format!("Update {package} in {manifest_path}");
    format!("{TITLE_PREFIX}{}", truncate_chars(&body, budget))
}

fn task_description(
    snapshot: &Value,
    ecosystem: &str,
    package: &str,
    manifest_path: &str,
    alerts: &[Value],
) -> String {
    let mut patched = alerts
        .iter()
        .map(|alert| field(alert, "first_patched_version"))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    patched.sort();
    patched.dedup();
    let patched = if patched.is_empty() {
        "not published".to_string()
    } else {
        patched.join(", ")
    };
    let repository = snapshot
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown repository");
    let mut out = format!(
        "This task was filed from a bounded, redacted Dependabot snapshot collected on the host before the task existed. The implementing agent does not need GitHub credentials.\n\n## Dependency bump\n\n- Repository: `{repository}`\n- Ecosystem: `{ecosystem}`\n- Package: `{package}`\n- Manifest path: `{manifest_path}`\n- First patched version: `{patched}`\n\n## Open alerts\n\n"
    );
    for alert in alerts {
        let ghsa = field(alert, "ghsa_id");
        let cve = field(alert, "cve_id");
        let identifier = match (ghsa.is_empty(), cve.is_empty()) {
            (false, false) => format!("{ghsa} / {cve}"),
            (false, true) => ghsa,
            (true, false) => cve,
            (true, true) => format!("alert #{}", field(alert, "number")),
        };
        out.push_str(&format!(
            "- **{}** — severity `{}`; {}; vulnerable range `{}`; first patched version `{}`; {}\n",
            identifier,
            display(&field(alert, "severity")),
            display(&field(alert, "summary")),
            display(&field(alert, "vulnerable_version_range")),
            display(&field(alert, "first_patched_version")),
            display(&field(alert, "html_url")),
        ));
    }
    if let Some(truncation) = snapshot.get("truncation") {
        out.push_str("\n## Collection bounds\n\n");
        out.push_str(&format!(
            "```json\n{}\n```\n",
            serde_json::to_string_pretty(truncation).unwrap_or_default()
        ));
    }
    out.push_str("\nUpdate the dependency and regenerate the lockfile through the repository's normal package-manager workflow. Verify that the manifest and lockfile agree and run the documented pre-handoff checks.\n");
    out
}

fn pull_request_mentions_package(pull_request: &Value, package: &str) -> bool {
    let package = package.to_ascii_lowercase();
    ["title", "body"].iter().any(|key| {
        pull_request
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|text| text.to_ascii_lowercase().contains(&package))
    })
}

fn open_task_for_key(runtime: &OrbitRuntime, key: &str) -> Result<Option<String>, OrbitError> {
    let tag = format!("{KEY_PREFIX}{key}");
    Ok(runtime
        .list_tasks_by_tags(std::slice::from_ref(&tag))?
        .into_iter()
        .find(|task| {
            !matches!(
                task.status,
                TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
            )
        })
        .map(|task| task.id))
}

fn severity_rank(value: &str) -> Option<u8> {
    match value {
        "low" => Some(1),
        "moderate" | "medium" => Some(2),
        "high" => Some(3),
        "critical" => Some(4),
        _ => None,
    }
}

fn priority_for_rank(rank: u8) -> TaskPriority {
    match rank {
        4.. => TaskPriority::Critical,
        3 => TaskPriority::High,
        _ => TaskPriority::Medium,
    }
}

fn digest(ecosystem: &str, package: &str, manifest_path: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [ecosystem, package, manifest_path] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
        .chars()
        .take(KEY_LEN)
        .collect()
}

fn bounded_u64(input: &Value, key: &str, default: u64, max: u64) -> Result<u64, OrbitError> {
    let Some(value) = input.get(key).filter(|value| !value.is_null()) else {
        return Ok(default);
    };
    let raw = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| OrbitError::InvalidInput(format!("input.{key} must be a positive integer")))?;
    if raw == 0 {
        return Err(OrbitError::InvalidInput(format!(
            "input.{key} must be greater than zero"
        )));
    }
    Ok(raw.min(max))
}

fn field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn display(value: &str) -> &str {
    if value.is_empty() { "unknown" } else { value }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect::<String>() + "…"
    }
}

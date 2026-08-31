use orbit_common::OrbitError;
use orbit_common::security::redaction::redact_all;
use serde_json::{Map, Value, json};

use super::query::{AlertQuery, DependabotQueries};
use super::{OUTCOME_CAPABILITY_UNAVAILABLE, OUTCOME_NO_OPEN_ALERTS, OUTCOME_OPEN_ALERTS};
use crate::executor::automation::ci::AuthStatus;
use crate::executor::automation::ci::bounded_u64;
use crate::executor::automation::ci::optional_input_string;

const SCHEMA_VERSION: u64 = 1;
const DEFAULT_LIMIT: u64 = 100;
const MAX_LIMIT: u64 = 100;

pub(super) fn collect<Q: DependabotQueries + ?Sized>(
    queries: &Q,
    input: &Value,
) -> Result<Value, OrbitError> {
    let limit = bounded_u64(input, "max_alerts", DEFAULT_LIMIT, MAX_LIMIT)?;
    let pr_limit = bounded_u64(input, "max_pull_requests", DEFAULT_LIMIT, MAX_LIMIT)?;
    let repo = optional_input_string(input, "repo");
    let auth = queries.auth_status();
    if !auth.usable() {
        let detail = auth.detail.clone();
        return capability_snapshot(auth, detail);
    }

    let repository = queries.repo_view(repo.as_deref())?;
    let alerts = match queries.open_alerts(repo.as_deref(), limit)? {
        AlertQuery::Alerts(alerts) => alerts,
        AlertQuery::CapabilityUnavailable(detail) => return capability_snapshot(auth, detail),
    };

    let mut query_errors = Vec::new();
    let pull_requests = match queries.open_dependabot_pull_requests(repo.as_deref(), pr_limit) {
        Ok(pull_requests) => pull_requests,
        Err(error) => {
            query_errors
                .push(json!({"query": "dependabot_pull_requests", "error": error.to_string()}));
            Vec::new()
        }
    };
    let alerts_at_cap = alerts.len() as u64 == limit;
    let prs_at_cap = pull_requests.len() as u64 == pr_limit;
    let notes = [
        alerts_at_cap.then(|| {
            format!("open alerts were listed at the {limit}-alert cap; further alerts may exist")
        }),
        prs_at_cap.then(|| {
            format!(
                "open Dependabot pull requests were listed at the {pr_limit}-PR cap; further PRs may exist"
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let snapshot = json!({
        "schema_version": SCHEMA_VERSION,
        "collected": true,
        "outcome_hint": if alerts.is_empty() { OUTCOME_NO_OPEN_ALERTS } else { OUTCOME_OPEN_ALERTS },
        "capability": auth.to_json(),
        "repository": repository,
        "open_alerts": alerts.into_iter().map(bound_alert).collect::<Vec<_>>(),
        "open_dependabot_pull_requests": pull_requests.into_iter().map(bound_pull_request).collect::<Vec<_>>(),
        "query_errors": query_errors,
        "truncation": {
            "alerts_limit": limit,
            "alerts_at_cap": alerts_at_cap,
            "pull_requests_limit": pr_limit,
            "pull_requests_at_cap": prs_at_cap,
            "notes": notes,
        },
        "collected_at": chrono::Utc::now().to_rfc3339(),
    });
    redact_snapshot(snapshot)
}

fn capability_snapshot(auth: AuthStatus, detail: String) -> Result<Value, OrbitError> {
    redact_snapshot(json!({
        "schema_version": SCHEMA_VERSION,
        "collected": false,
        "outcome_hint": OUTCOME_CAPABILITY_UNAVAILABLE,
        "capability": {
            "available": auth.available,
            "authenticated": auth.authenticated,
            "detail": detail,
        },
        "collected_at": chrono::Utc::now().to_rfc3339(),
    }))
}

fn bound_alert(mut alert: Value) -> Value {
    if let Some(object) = alert.as_object_mut() {
        for (key, value) in object.iter_mut() {
            if let Some(text) = value.as_str() {
                let max = if key == "summary" { 300 } else { 500 };
                *value = Value::String(one_line(text, max));
            }
        }
    }
    alert
}

fn bound_pull_request(mut pull_request: Value) -> Value {
    if let Some(object) = pull_request.as_object_mut() {
        bound_field(object, "title", 300);
        bound_field(object, "body", 1_000);
        bound_field(object, "url", 500);
        bound_field(object, "author", 100);
    }
    pull_request
}

fn bound_field(object: &mut Map<String, Value>, key: &str, max: usize) {
    if let Some(text) = object.get(key).and_then(Value::as_str) {
        object.insert(key.to_string(), Value::String(one_line(text, max)));
    }
}

fn one_line(text: &str, max: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max).collect()
}

fn redact_snapshot(snapshot: Value) -> Result<Value, OrbitError> {
    let encoded = serde_json::to_string(&snapshot).map_err(|error| {
        OrbitError::Execution(format!("failed to encode Dependabot snapshot: {error}"))
    })?;
    serde_json::from_str(&redact_all(&encoded)).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to decode redacted Dependabot snapshot: {error}"
        ))
    })
}

use orbit_common::OrbitError;
use serde_json::{Value, json};

use super::collect::collect;
use super::query::{AlertQuery, DependabotQueries, classify_alert_failure};
use crate::executor::automation::ci::AuthStatus;

struct ScriptedQueries {
    dependabot: AlertQuery,
    code_scanning: AlertQuery,
    secret_scanning: AlertQuery,
    locations: Vec<Value>,
    location_error: bool,
}

fn clone_query(query: &AlertQuery) -> AlertQuery {
    match query {
        AlertQuery::Alerts(alerts) => AlertQuery::Alerts(alerts.clone()),
        AlertQuery::CapabilityUnavailable(detail) => {
            AlertQuery::CapabilityUnavailable(detail.clone())
        }
    }
}

fn scripted(dependabot: AlertQuery) -> ScriptedQueries {
    ScriptedQueries {
        dependabot,
        code_scanning: AlertQuery::Alerts(Vec::new()),
        secret_scanning: AlertQuery::Alerts(Vec::new()),
        locations: Vec::new(),
        location_error: false,
    }
}

impl DependabotQueries for ScriptedQueries {
    fn auth_status(&self) -> AuthStatus {
        AuthStatus {
            available: true,
            authenticated: true,
            detail: "authenticated".to_string(),
        }
    }

    fn repo_view(&self, _repo: Option<&str>) -> Result<Value, OrbitError> {
        Ok(json!({"name": "orbit", "full_name": "acme/orbit", "default_branch": "main"}))
    }

    fn open_alerts(&self, _repo: Option<&str>, _limit: u64) -> Result<AlertQuery, OrbitError> {
        Ok(clone_query(&self.dependabot))
    }

    fn open_code_scanning_alerts(
        &self,
        _repo: Option<&str>,
        _limit: u64,
    ) -> Result<AlertQuery, OrbitError> {
        Ok(clone_query(&self.code_scanning))
    }

    fn open_secret_scanning_alerts(
        &self,
        _repo: Option<&str>,
        _limit: u64,
    ) -> Result<AlertQuery, OrbitError> {
        Ok(clone_query(&self.secret_scanning))
    }

    fn secret_scanning_locations(
        &self,
        _repo: Option<&str>,
        _alert_number: u64,
        _limit: u64,
    ) -> Result<Vec<Value>, OrbitError> {
        if self.location_error {
            Err(OrbitError::Execution(
                "permission unavailable for locations".to_string(),
            ))
        } else {
            Ok(self.locations.clone())
        }
    }

    fn open_dependabot_pull_requests(
        &self,
        _repo: Option<&str>,
        _limit: u64,
    ) -> Result<Vec<Value>, OrbitError> {
        Ok(Vec::new())
    }
}

#[test]
fn http_403_and_404_are_distinct_capability_outcomes_not_no_alerts() {
    for (stderr, expected) in [
        (
            "gh: Resource not accessible by integration (HTTP 403)",
            "required repository security permission",
        ),
        (
            "gh: Code scanning is not enabled (HTTP 404)",
            "disabled or unavailable",
        ),
    ] {
        let detail = classify_alert_failure("Code scanning alerts", stderr);
        assert!(detail.contains(expected));
        let mut queries = scripted(AlertQuery::Alerts(Vec::new()));
        queries.code_scanning = AlertQuery::CapabilityUnavailable(detail);
        let output = collect(&queries, &json!({})).expect("collect capability snapshot");
        assert_eq!(output["code_scanning"]["collected"], json!(false));
        assert_eq!(
            output["code_scanning"]["outcome_hint"],
            json!("capability_unavailable")
        );
        assert_ne!(
            output["code_scanning"]["outcome_hint"],
            json!("no_open_alerts")
        );
        assert_eq!(output["collected"], json!(true));
        assert_eq!(output["collection_status"], json!("partially_collected"));
    }
}

#[test]
fn empty_success_is_the_only_no_open_alerts_outcome() {
    let output = collect(&scripted(AlertQuery::Alerts(Vec::new())), &json!({}))
        .expect("collect empty snapshot");
    assert_eq!(output["collected"], json!(true));
    assert_eq!(output["outcome_hint"], json!("no_open_alerts"));
    assert_eq!(output["collection_status"], json!("fully_collected"));
}

#[test]
fn secret_value_is_structurally_absent_and_location_truncation_is_explicit() {
    const SENTINEL: &str = "orbit-sentinel-credential-7f21e6";
    let mut queries = scripted(AlertQuery::Alerts(Vec::new()));
    let projected = orbit_tools::github_cli::project_secret_scanning_alert(&json!({
        "number": 41,
        "state": "open",
        "secret_type": "example_token",
        "secret_type_display_name": "Example token",
        "secret": SENTINEL,
        "validity": "active",
        "publicly_leaked": true,
        "html_url": "https://github.test/acme/orbit/security/secret-scanning/41"
    }));
    queries.secret_scanning = AlertQuery::Alerts(vec![projected]);
    queries.locations = vec![json!({
        "type": "commit", "path": "config/dev.env", "start_line": 4,
        "end_line": 4, "commit_sha": "abc123", "commit_url": "https://github.test/commit/abc123"
    })];
    let output =
        collect(&queries, &json!({"max_secret_locations": 1})).expect("collect secret snapshot");
    let encoded = serde_json::to_string(&output).expect("encode snapshot");
    assert!(!encoded.contains(SENTINEL));
    assert_eq!(
        output["secret_scanning"]["open_alerts"][0]["locations"][0]["path"],
        "config/dev.env"
    );
    assert_eq!(
        output["secret_scanning"]["truncation"]["locations_at_cap_alerts"],
        json!([41])
    );
}

#[test]
fn location_permission_failure_is_partial_not_zero_findings() {
    let mut queries = scripted(AlertQuery::Alerts(Vec::new()));
    queries.secret_scanning = AlertQuery::Alerts(vec![json!({
        "number": 42, "secret_type": "example", "validity": "unknown"
    })]);
    queries.location_error = true;
    let output = collect(&queries, &json!({})).expect("collect partial snapshot");
    assert_eq!(
        output["secret_scanning"]["collection_status"],
        "partially_collected"
    );
    assert_eq!(output["secret_scanning"]["outcome_hint"], "open_alerts");
    assert_eq!(
        output["secret_scanning"]["query_errors"]
            .as_array()
            .expect("errors")
            .len(),
        1
    );
}

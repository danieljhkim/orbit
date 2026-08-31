use orbit_common::OrbitError;
use serde_json::{Value, json};

use super::collect::collect;
use super::query::{AlertQuery, DependabotQueries, classify_alert_failure};
use crate::executor::automation::ci::AuthStatus;

struct ScriptedQueries {
    alerts: AlertQuery,
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
        Ok(match &self.alerts {
            AlertQuery::Alerts(alerts) => AlertQuery::Alerts(alerts.clone()),
            AlertQuery::CapabilityUnavailable(detail) => {
                AlertQuery::CapabilityUnavailable(detail.clone())
            }
        })
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
            "security_events",
        ),
        (
            "gh: Dependabot alerts are not enabled (HTTP 404)",
            "disabled or unavailable",
        ),
    ] {
        let detail = classify_alert_failure(stderr).expect("classified capability response");
        assert!(detail.contains(expected));
        let output = collect(
            &ScriptedQueries {
                alerts: AlertQuery::CapabilityUnavailable(detail),
            },
            &json!({}),
        )
        .expect("collect capability snapshot");
        assert_eq!(output["collected"], json!(false));
        assert_eq!(output["outcome_hint"], json!("capability_unavailable"));
        assert_ne!(output["outcome_hint"], json!("no_open_alerts"));
    }
}

#[test]
fn empty_success_is_the_only_no_open_alerts_outcome() {
    let output = collect(
        &ScriptedQueries {
            alerts: AlertQuery::Alerts(Vec::new()),
        },
        &json!({}),
    )
    .expect("collect empty snapshot");
    assert_eq!(output["collected"], json!(true));
    assert_eq!(output["outcome_hint"], json!("no_open_alerts"));
}

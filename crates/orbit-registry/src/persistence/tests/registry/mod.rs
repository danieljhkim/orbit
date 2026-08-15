use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{
    ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1, HostNameResolution,
    HostRegistration, HostStatus, ProjectionFreshness, WorkspacePresenceDeclaration,
};

use super::super::RegistryStore;

fn registration(machine_id: &str, host_id: &str, labels: &[&str]) -> HostRegistration {
    HostRegistration {
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
    }
}

fn error_text(result: Result<impl std::fmt::Debug, orbit_common::types::OrbitError>) -> String {
    result.expect_err("operation must fail").to_string()
}

fn execution_profile(
    workspace_id: &str,
    owner_machine_id: &str,
    observed_at: chrono::DateTime<Utc>,
) -> ExecutionProfileV1 {
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: workspace_id.to_string(),
        owner_machine_id: owner_machine_id.to_string(),
        observed_at,
        config_digest: String::new(),
        default_crew: "sol".to_string(),
        crews: vec![ExecutionProfileCrewV1 {
            name: "sol".to_string(),
            provider: "codex".to_string(),
            model: "gpt-test".to_string(),
            backend: "cli".to_string(),
            description: Some("Systems implementation".to_string()),
            tags: vec!["hard".to_string()],
        }],
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    profile
}

mod hosts;
mod revision;
mod workspaces;

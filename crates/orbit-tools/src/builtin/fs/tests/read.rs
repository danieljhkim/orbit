use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::{
    FsOperation, OrbitError, PolicyDef, ResourceKind, parse_policy_resource,
};
use orbit_policy::PolicyEngine;
use serde_json::json;

use super::super::read::FsReadTool;
use crate::{Tool, ToolContext};

const DEFAULT_POLICY: &str = include_str!("../../../../../orbit-core/assets/policies/default.yaml");

fn default_policy_engine() -> Arc<PolicyEngine> {
    let resource =
        parse_policy_resource(DEFAULT_POLICY, "seeded default policy").expect("parse policy");
    assert_eq!(resource.kind, ResourceKind::Policy);

    let now = Utc::now();
    let def = PolicyDef {
        name: resource.metadata.name,
        description: resource.spec.description,
        deny_read: resource.spec.deny_read,
        deny_modify: resource.spec.deny_modify,
        fs_profiles: resource.spec.fs_profiles,
        created_at: now,
        updated_at: now,
    };

    Arc::new(PolicyEngine::from_def(&def).expect("policy engine"))
}

#[test]
fn fs_read_denies_dotenv_local_with_default_policy() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let dotenv_path = workspace.path().join(".env.local");
    std::fs::write(&dotenv_path, "API_KEY=secret\n").expect("write dotenv");

    let ctx = ToolContext {
        workspace_root: Some(workspace.path().to_path_buf()),
        policy_engine: Some(default_policy_engine()),
        fs_profile: Some("implementer".to_string()),
        ..ToolContext::default()
    };

    let error = FsReadTool
        .execute(
            &ctx,
            json!({
                "path": dotenv_path.display().to_string(),
            }),
        )
        .expect_err("default policy should deny .env.local reads");

    let OrbitError::PolicyDenied(message) = error else {
        panic!("expected PolicyDenied error");
    };
    assert!(message.contains(".env.local"));
}

#[test]
fn default_policy_denies_dotenv_variants_for_read_and_modify() {
    let engine = default_policy_engine();

    for path in [
        ".env",
        ".env.local",
        ".env.production",
        "foo/secrets.env.bak",
    ] {
        for operation in [FsOperation::Read, FsOperation::Modify] {
            let result = engine
                .check("implementer", operation, path)
                .expect("policy check");

            assert!(
                !result.allowed,
                "{operation:?} should deny `{path}` under the default policy"
            );
            assert!(
                result.matched_rule.contains(".env"),
                "matched rule should identify a dotenv deny rule, got `{}`",
                result.matched_rule
            );
        }
    }
}

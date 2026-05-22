//! Sibling tests for `runtime_host.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::collections::HashMap;

use chrono::Utc;
use orbit_common::types::{Activity, ExecutorDef, ExecutorType};
use orbit_engine::RuntimeHost;
use serde_json::json;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::OrbitRuntime;

#[test]
fn planning_duel_invoke_activity_uses_direct_agent_bridge() {
    let seed_runtime = OrbitRuntime::in_memory().expect("build runtime");
    let root = seed_runtime.data_root();
    let fake_agent = root.join("fake-agent.sh");
    std::fs::write(
        &fake_agent,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"ok\":true},\"error\":null}'\n",
    )
    .expect("write fake agent");
    #[cfg(unix)]
    {
        let mut permissions = std::fs::metadata(&fake_agent)
            .expect("fake agent metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_agent, permissions).expect("chmod fake agent");
    }

    let now = Utc::now();
    seed_runtime
        .upsert_executor_def(&ExecutorDef {
            name: "codex".to_string(),
            executor_type: ExecutorType::DirectAgent,
            command: Some(fake_agent.display().to_string()),
            args: Vec::new(),
            stdout_format: None,
            model_pair_override: None,
            model_flag: None,
            timeout_seconds: None,
            env: HashMap::new(),
            sandbox: None,
            allow_fallback: false,
            created_at: now,
            updated_at: now,
        })
        .expect("seed fake direct-agent executor");
    let runtime = OrbitRuntime::from_roots(&root, &root).expect("reload runtime");

    let result = runtime
        .invoke_activity(
            planning_duel_activity("propose_duel_plan"),
            "codex",
            Some("test-model"),
            json!({}),
            5,
            false,
        )
        .expect("planning duel activity should invoke through bridge");

    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.response_json, None);
}

#[test]
fn invoke_activity_still_rejects_non_planning_duel_v1_activities() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = runtime
        .invoke_activity(
            planning_duel_activity("legacy_activity"),
            "codex",
            None,
            json!({}),
            5,
            false,
        )
        .expect_err("unrelated v1 activity should remain retired");

    assert!(
        err.to_string().contains("v1 invoke_activity is retired"),
        "unexpected error: {err}"
    );
}

fn planning_duel_activity(id: &str) -> Activity {
    let now = Utc::now();
    Activity {
        id: id.to_string(),
        spec_type: "agent_invoke".to_string(),
        description: "test planning duel activity".to_string(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        spec_config: json!({
            "instruction": "Return a success envelope."
        }),
        tools: Vec::new(),
        proc_allowed_programs: Vec::new(),
        executor: None,
        workspace_path: None,
        created_by: Some("test".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

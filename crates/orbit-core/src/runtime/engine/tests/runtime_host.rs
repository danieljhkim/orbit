//! Sibling tests for `runtime_host.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::collections::HashMap;

use chrono::Utc;
use orbit_common::types::{ExecutorDef, ExecutorType};
use orbit_engine::{
    RuntimeHost, V2AgentDispatchOverride, V2DispatchInput, V2RuntimeHost, dispatch_v2_activity,
};
use orbit_store::InvocationQuery;
use serde_json::json;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::OrbitRuntime;
use crate::command::activity::seed_default_activities;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

#[test]
fn planning_duel_v2_dispatch_preserves_slot_model_and_task_attribution() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_default_activities(&runtime.data_root().join("resources/activities"), true)
        .expect("seed v2 activities");
    let task = runtime
        .add_task(TaskAddParams {
            title: "v2 planning duel dispatch".to_string(),
            description: "Exercise a real task through the planner v2 activity.".to_string(),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add task");
    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("Use the v2 path.".to_string()),
                ..Default::default()
            },
        )
        .expect("give task a plan");

    let now = Utc::now();
    for provider in ["codex", "claude", "gemini"] {
        let fake_agent = runtime.data_root().join(provider);
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
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: provider.to_string(),
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
    }

    let run_id = "planning-duel-v2-test";
    let audit = RuntimeHost::v2_audit_writer(&runtime, run_id).expect("v2 audit writer");
    for (activity_name, slot, provider, model) in [
        (
            "propose_duel_plan",
            "planner_a",
            "codex",
            "duel-planner-a-model",
        ),
        (
            "propose_duel_plan",
            "planner_b",
            "claude",
            "duel-planner-b-model",
        ),
        (
            "arbitrate_duel_plan",
            "arbiter",
            "gemini",
            "duel-arbiter-model",
        ),
    ] {
        let activity =
            RuntimeHost::v2_activity(&runtime, activity_name).expect("load seeded duel activity");
        let input = json!({
            "task_id": task.id,
            "planning_duel_slot": slot,
        });
        let outcome = dispatch_v2_activity(V2DispatchInput {
            activity_name,
            spec: &activity.spec,
            fs_profile: activity.fs_profile.as_deref(),
            input: input.clone(),
            audit: audit.clone(),
            run_id,
            agent_override: Some(V2AgentDispatchOverride {
                provider,
                model: Some(model),
            }),
            host: Some(&runtime),
        })
        .expect("dispatch duel v2 activity");

        assert!(outcome.success, "{:?}", outcome.message);
        let invocation = outcome.invocation.expect("duel invocation trace");
        assert_eq!(invocation.provider, provider);
        assert_eq!(invocation.model.as_deref(), Some(model));
        V2RuntimeHost::persist_invocation_trace(
            &runtime,
            run_id,
            activity_name,
            &invocation.provider,
            invocation.model.as_deref(),
            &input,
            &invocation.trace,
        )
        .expect("persist v2 invocation trace");
    }

    let rows = runtime
        .invocation_records(InvocationQuery {
            job_run_id: Some(run_id.to_string()),
            ..InvocationQuery::default()
        })
        .expect("read invocation telemetry");
    assert_eq!(rows.len(), 3);
    for (slot, agent, model, activity_id) in [
        (
            "planner_a",
            "codex",
            "duel-planner-a-model",
            "propose_duel_plan",
        ),
        (
            "planner_b",
            "claude",
            "duel-planner-b-model",
            "propose_duel_plan",
        ),
        (
            "arbiter",
            "gemini",
            "duel-arbiter-model",
            "arbitrate_duel_plan",
        ),
    ] {
        let row = rows
            .iter()
            .find(|row| row.slot.map(|row_slot| row_slot.as_str()) == Some(slot))
            .expect("slot telemetry row");
        assert_eq!(row.agent, agent);
        assert_eq!(row.model.as_deref(), Some(model));
        assert_eq!(row.activity_id, activity_id);
        assert_eq!(row.task_ids, [task.id.clone()]);
    }
}

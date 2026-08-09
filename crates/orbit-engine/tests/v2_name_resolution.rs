#![allow(missing_docs)]
// Integration fixtures exercise public behavior and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! Name-resolution integration coverage — T20260418-2019.
//!
//! Exercises:
//!   A) `V2ActivityCatalog::load_dir` picks up the four new v2 activities
//!      (`agent_assess_diff`, `agent_apply_fixes`, `promote_agent_main`,
//!      `revert_on_red`) and skips v1 assets silently.
//!   B) `resolve_job_target_refs` rewrites `target: activity:<name>` refs
//!      into inline `TargetStep`s using the catalog.
//!   C) A round-trip through backend resolution + §3.2 loader rejection
//!      works on the resolved job — unknown refs surface a structural
//!      error, not a silent no-op.
//!   D) Loading the new `task_pipeline.yaml` sample produces a job with
//!      `TargetRef`s that point at the bundled v2 activities plus the
//!      not-yet-ported activities (which is the expected partial state of
//!      Phase 4). Only the resolvable refs rewrite; unresolved ones are
//!      reported by the resolver.
//!
//! Runs under `cargo nextest run -p orbit-engine --test v2_name_resolution`.

use std::path::PathBuf;
use std::sync::Arc;

use orbit_common::types::JobScheduleState;
use orbit_common::types::activity_job::{
    ActivityV2, ActivityV2Spec, Backend, JobKind, JobV2, JobV2Step, JobV2StepBody, LoopBlock,
    Provider, ResolveError, TargetRef, V2ActivityCatalog, load_job_asset, resolve_job_backends,
    resolve_job_target_refs, validate_job_loop_session_backends,
};
use orbit_engine::{
    DispatchError, ResolvedCliExecutor, V2AuditWriter, V2DispatchInput, V2RuntimeHost,
    dispatch_v2_activity,
};
use serde_json::Value;

#[test]
fn name_resolution_regressions() -> Result<(), Box<dyn std::error::Error>> {
    scenario_a_catalog_loads_new_activities()?;
    scenario_b_target_ref_resolves()?;
    scenario_c_unknown_ref_is_structural_error()?;
    scenario_d_pipeline_yaml_partial_resolution()?;
    scenario_e_backend_rejection_runs_after_resolution()?;
    scenario_f_deterministic_activities_dispatch()?;

    Ok(())
}

fn scenario_a_catalog_loads_new_activities() -> Result<(), Box<dyn std::error::Error>> {
    println!("  A) catalog retains supported examples and excludes retired promotion");
    let mut catalog = V2ActivityCatalog::new();
    let dir = repo_root().join("crates/orbit-core/assets/activities");
    catalog.load_dir(&dir)?;

    for name in ["agent_assess_diff", "agent_apply_fixes", "revert_on_red"] {
        assert!(
            catalog.get(name).is_some(),
            "catalog missing new activity `{}` (present: {:?})",
            name,
            catalog.names().collect::<Vec<_>>()
        );
    }
    // Pinning: cross-iteration assessment must be backend: http.
    let assessor = catalog.get("agent_assess_diff").expect("present");
    let ActivityV2Spec::AgentLoop(spec) = &assessor.spec else {
        panic!("agent_assess_diff should be agent_loop");
    };
    assert_eq!(spec.backend, Backend::Http, "assessor must pin http");
    assert_eq!(spec.provider, Provider::Claude);

    // Fixer is auto — no `session:` binding in the activity itself.
    let fixer = catalog.get("agent_apply_fixes").expect("present");
    let ActivityV2Spec::AgentLoop(fixer_spec) = &fixer.spec else {
        panic!("agent_apply_fixes should be agent_loop");
    };
    assert_eq!(fixer_spec.backend, Backend::Auto);

    let revert = catalog.get("revert_on_red").expect("present");
    assert!(matches!(&revert.spec, ActivityV2Spec::Deterministic(_)));
    assert!(
        catalog.get("promote_agent_main").is_none(),
        "retired promotion must not remain in the activity catalog"
    );

    println!(
        "    loaded {} activities total (filter selects retained surface)",
        catalog.len()
    );
    Ok(())
}

fn scenario_b_target_ref_resolves() -> Result<(), Box<dyn std::error::Error>> {
    println!("  B) resolve_job_target_refs rewrites named refs to inline specs");
    let catalog = load_reference_catalog()?;

    let mut job = synthetic_job_using_ref("agent_assess_diff");
    resolve_job_target_refs(&mut job, &catalog)?;

    // After resolution the body must be an inline Target, not a TargetRef.
    let JobV2StepBody::Target(t) = &job.steps[0].body else {
        panic!(
            "expected Target after resolution, got {:?}",
            job.steps[0].body
        );
    };
    let ActivityV2Spec::AgentLoop(spec) = &t.spec else {
        panic!("expected agent_loop spec");
    };
    assert_eq!(spec.backend, Backend::Http);
    assert_eq!(t.session.as_deref(), Some("assessor"));
    println!("    resolved ref → inline Target with session=assessor");
    Ok(())
}

fn scenario_c_unknown_ref_is_structural_error() -> Result<(), Box<dyn std::error::Error>> {
    println!("  C) unknown activity name surfaces ResolveError structurally");
    let catalog = load_reference_catalog()?;
    let mut job = synthetic_job_using_ref("does_not_exist");
    let err = resolve_job_target_refs(&mut job, &catalog).expect_err("expected error");
    match err {
        ResolveError::ActivityNotInCatalog { step_id, name } => {
            assert_eq!(step_id, "the_step");
            assert_eq!(name, "does_not_exist");
            println!("    got ActivityNotInCatalog for `{}`", name);
        }
        other => panic!("wrong error: {other:?}"),
    }
    Ok(())
}

fn scenario_d_pipeline_yaml_partial_resolution() -> Result<(), Box<dyn std::error::Error>> {
    println!("  D) task_pipeline.yaml exposes retired promotion structurally");
    let yaml_path = repo_root().join("crates/orbit-core/assets/jobs/examples/task_pipeline.yaml");
    let yaml = std::fs::read_to_string(&yaml_path)?;
    let asset = load_job_asset(&yaml)?;

    // Confirm the parse produced TargetRefs (not inline specs) throughout.
    let ref_count = count_target_refs(&asset.spec);
    assert!(
        ref_count >= 8,
        "expected at least 8 TargetRefs in pipeline, got {}",
        ref_count
    );

    // The post-sweep catalog resolves the live pipeline surface but leaves
    // retired promotion unresolved rather than silently treating it as live.
    let catalog = load_reference_catalog()?;
    let mut partial = asset.spec.clone();
    let err = resolve_job_target_refs(&mut partial, &catalog);
    match err {
        Err(ResolveError::ActivityNotInCatalog { name, .. }) => {
            assert_eq!(name, "promote_agent_main");
            println!("    retired promotion remains a structural resolution error");
        }
        Ok(_) => panic!("expected retired promotion to remain unresolved"),
        Err(other) => panic!("wrong error: {other:?}"),
    }

    // A synthetic promotion entry proves all remaining targets resolve; it
    // must never be shipped as a real activity until the action exists.
    let mut catalog_with_stubs = catalog;
    catalog_with_stubs.insert(
        "promote_agent_main",
        stub_deterministic_activity("promote_agent_main"),
    );
    let mut full = asset.spec.clone();
    resolve_job_target_refs(&mut full, &catalog_with_stubs)?;
    assert_eq!(
        count_target_refs(&full),
        0,
        "every TargetRef should be resolved after stubs land"
    );
    println!("    all refs resolve with stubs in place");
    Ok(())
}

fn scenario_e_backend_rejection_runs_after_resolution() -> Result<(), Box<dyn std::error::Error>> {
    println!("  E) §3.2 rejection operates on resolved specs — assessor session survives");
    let catalog = load_reference_catalog()?;
    let mut job = pipeline_with_assessor_loop();
    resolve_job_target_refs(&mut job, &catalog)?;
    // After resolution, the assessor step has Backend::Http pinned (from
    // the asset file), so backend auto-resolution + §3.2 validator pass.
    resolve_job_backends(&mut job, Backend::Http);
    validate_job_loop_session_backends(&job, "synthetic")?;
    println!("    loop+session+http assessor accepted by validator");

    // Flipping the assessor activity to cli in-catalog triggers the §3.2
    // rejection once resolution inlines the spec.
    let mut cli_catalog = V2ActivityCatalog::new();
    let mut assessor_cli = catalog.get("agent_assess_diff").expect("present").clone();
    if let ActivityV2Spec::AgentLoop(spec) = &mut assessor_cli.spec {
        spec.backend = Backend::Cli;
    }
    cli_catalog.insert("agent_assess_diff", assessor_cli);
    let mut job = pipeline_with_assessor_loop();
    resolve_job_target_refs(&mut job, &cli_catalog)?;
    resolve_job_backends(&mut job, Backend::Cli);
    let err =
        validate_job_loop_session_backends(&job, "synthetic").expect_err("expected §3.2 rejection");
    println!(
        "    flipping assessor backend → cli triggers rejection: {}",
        err
    );
    Ok(())
}

/// F: the retired `revert_on_red` example still parses, but its deleted action
/// must fail loudly rather than becoming a skipped-success path.
fn scenario_f_deterministic_activities_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    println!("  F) retired revert_on_red action is rejected structurally");
    let catalog = load_reference_catalog()?;
    let host = PipelineHost;

    let activity = catalog.get("revert_on_red").expect("present");
    let tmp = tempfile::tempdir()?;
    let writer = build_writer(tmp.path(), "name-resolution-revert")?;
    let err = dispatch_v2_activity(V2DispatchInput {
        activity_name: "revert_on_red",
        spec: &activity.spec,
        fs_profile: activity.fs_profile.as_deref(),
        input: serde_json::json!({
            "commit_sha": "deadbeef",
            "branch": "agent-main",
            "reason": "coverage",
        }),
        audit: writer,
        run_id: "name-resolution-revert",
        host: Some(&host),
    })
    .expect_err("retired action must be rejected");
    assert!(
        matches!(err, DispatchError::DeterministicActionNotRegistered(action) if action == "revert_on_red")
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_writer(
    root: &std::path::Path,
    run_id: &str,
) -> Result<Arc<V2AuditWriter>, Box<dyn std::error::Error>> {
    let audit_root = root.join("audit");
    std::fs::create_dir_all(&audit_root)?;
    let writer = V2AuditWriter::with_disk_sinks(
        &audit_root,
        orbit_store::Store::open_in_memory()?,
        "ws_smoke",
        run_id,
        "smoke".to_string(),
        None,
    )?;
    Ok(writer)
}

/// Host that models the post-sweep deterministic action surface.
struct PipelineHost;

impl V2RuntimeHost for PipelineHost {
    fn run_deterministic(
        &self,
        action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            action.to_string(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "PipelineHost has no credentials".into(),
        ))
    }

    fn resolve_cli_executor(&self, _provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "PipelineHost has no CLI mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        _proc_allowed_programs: Option<&[String]>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }
}

fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn load_reference_catalog() -> Result<V2ActivityCatalog, Box<dyn std::error::Error>> {
    let mut catalog = V2ActivityCatalog::new();
    let dir = repo_root().join("crates/orbit-core/assets/activities");
    catalog.load_dir(&dir)?;
    Ok(catalog)
}

fn synthetic_job_using_ref(target_name: &str) -> JobV2 {
    JobV2 {
        state: JobScheduleState::Enabled,
        default_input: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        failure_activity: None,
        resolved_failure_activity: None,
        max_active_runs: 1,
        kind: JobKind::Workflow,
        steps: vec![JobV2Step {
            id: "the_step".to_string(),
            when: None,
            retry: None,
            recovery_activity: None,
            resolved_recovery_activity: None,
            body: JobV2StepBody::TargetRef(TargetRef {
                target: format!("activity:{}", target_name),
                default_input: None,
                timeout_seconds: 0,
                session: Some("assessor".to_string()),
                role: None,
            }),
        }],
    }
}

fn pipeline_with_assessor_loop() -> JobV2 {
    let assess_step = JobV2Step {
        id: "assess".to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::TargetRef(TargetRef {
            target: "activity:agent_assess_diff".to_string(),
            default_input: None,
            timeout_seconds: 0,
            session: Some("assessor".to_string()),
            role: None,
        }),
    };
    JobV2 {
        state: JobScheduleState::Enabled,
        default_input: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        failure_activity: None,
        resolved_failure_activity: None,
        max_active_runs: 1,
        kind: JobKind::Workflow,
        steps: vec![JobV2Step {
            id: "assess_fix".to_string(),
            when: None,
            retry: None,
            recovery_activity: None,
            resolved_recovery_activity: None,
            body: JobV2StepBody::Loop {
                loop_: LoopBlock {
                    items: None,
                    max_iterations: 3,
                    break_when: None,
                    steps: vec![assess_step],
                },
            },
        }],
    }
}

fn stub_deterministic_activity(name: &str) -> ActivityV2 {
    ActivityV2 {
        description: format!("stub for `{name}` — pending v1 port"),
        input_schema_json: serde_json::Value::Null,
        output_schema_json: serde_json::Value::Null,
        fs_profile: None,
        spec: ActivityV2Spec::Deterministic(orbit_common::types::activity_job::DeterministicSpec {
            action: "noop".to_string(),
            config: serde_json::Value::Null,
        }),
    }
}

// Walk a job counting remaining TargetRefs — anything >0 after resolution
// means Phase 4 hasn't finished porting that activity.
fn count_target_refs(job: &JobV2) -> usize {
    fn count_step(step: &JobV2Step) -> usize {
        match &step.body {
            JobV2StepBody::TargetRef(_) => 1,
            JobV2StepBody::Target(_) => 0,
            JobV2StepBody::Parallel { parallel } => parallel.branches.iter().map(count_step).sum(),
            JobV2StepBody::FanOut { fan_out, .. } => count_step(&fan_out.worker),
            JobV2StepBody::Loop { loop_ } => loop_.steps.iter().map(count_step).sum(),
        }
    }
    job.steps.iter().map(count_step).sum()
}

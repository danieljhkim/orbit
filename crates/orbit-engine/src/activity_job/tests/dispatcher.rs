#![allow(missing_docs)]

use serde_json::{Map, Value, json};

use super::super::dispatcher::agent_loop_output_from_final_message;
use super::super::dispatcher::{V2AgentDispatchOverride, overridden_agent_spec};

#[test]
fn agent_loop_output_exposes_structured_final_message_fields() {
    let mut metadata = Map::new();
    metadata.insert(
        "final_message".to_string(),
        Value::String("raw".to_string()),
    );

    let output = agent_loop_output_from_final_message(
        r#"{"cycle_notes":"dispatched one","dispatched_run_ids":["jrun-1"]}"#,
        metadata,
    );

    assert_eq!(output["cycle_notes"], json!("dispatched one"));
    assert_eq!(output["dispatched_run_ids"], json!(["jrun-1"]));
    assert_eq!(output["final_message"], json!("raw"));
}

#[test]
fn agent_loop_output_unwraps_response_envelope_result() {
    let output = agent_loop_output_from_final_message(
        r#"{"schemaVersion":1,"status":"success","result":{"dispatched_run_ids":[]}}"#,
        Map::new(),
    );

    assert_eq!(output["dispatched_run_ids"], json!([]));
    assert!(output.get("schemaVersion").is_none());
}

#[test]
fn dispatch_error_retryability_classification_table() {
    // ORB-10006: retryable-vs-permanent classification consumed by the step
    // retry wrapper. Permanent (non-retryable) errors fail fast; transient
    // ones burn retry attempts.
    use super::super::dispatcher::DispatchError;

    let permanent: Vec<DispatchError> = vec![
        DispatchError::ToolDenied {
            tool_name: "fs.write".into(),
            iteration: 1,
        },
        DispatchError::DeterministicActionNotRegistered("nope".into()),
        DispatchError::JobValidation("bad".into()),
        DispatchError::RetryConfigInvalid {
            step_id: "s".into(),
            field: "max_attempts",
            value: 0,
            invariant: "max_attempts >= 1".into(),
        },
        DispatchError::HostRequired("host"),
        DispatchError::UnwiredHttpTransport {
            provider: "claude".into(),
        },
        DispatchError::UnresolvedAutoBackend {
            step_id: "s".into(),
        },
        DispatchError::CliInvocationPermanent("agent config: bad model".into()),
        DispatchError::WorktreeIntegrity {
            code: "worktree_escape",
            diagnostic: r#"{"task_id":"ORB-1"}"#.into(),
        },
    ];
    for err in &permanent {
        assert!(err.is_non_retryable(), "expected non-retryable: {err:?}");
    }

    let transient: Vec<DispatchError> = vec![
        DispatchError::CliInvocationFailed("spawn claude: EAGAIN".into()),
        DispatchError::AgentLoopFailed("overloaded".into()),
        DispatchError::DeterministicActionFailed {
            action: "a".into(),
            message: "flaky".into(),
        },
        DispatchError::JobExecution("executor hiccup".into()),
        DispatchError::AuditFailed("sink".into()),
    ];
    for err in &transient {
        assert!(!err.is_non_retryable(), "expected retryable: {err:?}");
    }
}

#[test]
fn dispatch_error_to_orbit_keeps_validation_variant_and_buckets_the_rest() {
    use orbit_common::types::OrbitError;

    use super::super::dispatcher::{DispatchError, dispatch_error_to_orbit};

    assert!(matches!(
        dispatch_error_to_orbit(DispatchError::JobValidation("bad spec".into())),
        OrbitError::JobValidation(m) if m == "bad spec"
    ));

    let other = DispatchError::AgentLoopFailed("overloaded".into());
    let expected = other.to_string();
    assert!(matches!(
        dispatch_error_to_orbit(other),
        OrbitError::InvalidInput(m) if m == expected
    ));
}

#[test]
fn per_dispatch_agent_override_sets_provider_model_and_renders_family() {
    use orbit_common::types::activity_job::{
        ActivityV2Spec, AgentLoopSpec, Backend, OnDenial, Provider,
    };

    let spec = ActivityV2Spec::AgentLoop(AgentLoopSpec {
        instruction: "family={{agent_family}}".to_string(),
        backend: Backend::Cli,
        provider: Provider::Claude,
        model: Some("asset-default".to_string()),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        max_iterations: 20,
        wall_clock_timeout_seconds: 300,
        require_response_envelope: false,
        require_completion_envelope: true,
        role: None,
        proc_allowed_programs: None,
    });
    let overridden = overridden_agent_spec(
        &spec,
        Some(V2AgentDispatchOverride {
            provider: "codex",
            model: Some("gpt-duel-a"),
        }),
    )
    .expect("apply override")
    .expect("overridden spec");

    let ActivityV2Spec::AgentLoop(agent) = overridden else {
        panic!("expected agent-loop override");
    };
    assert_eq!(agent.provider, Provider::Codex);
    assert_eq!(agent.model.as_deref(), Some("gpt-duel-a"));
    assert_eq!(agent.instruction, "family=codex");

    let ActivityV2Spec::AgentLoop(original) = spec else {
        panic!("expected original agent-loop spec");
    };
    assert_eq!(original.provider, Provider::Claude);
    assert_eq!(original.model.as_deref(), Some("asset-default"));
    assert_eq!(original.instruction, "family={{agent_family}}");
}

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use super::*;

pub(super) fn run_target(
    step: &JobV2Step,
    t: &TargetStep,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let tctx = ctx.template_ctx();
    let rendered_input = render_input(t.default_input.as_ref(), &ctx.input, &tctx)?;

    // Agent settings override — `step.role` (TargetStep) wins over
    // `activity.role` (AgentLoopSpec) when both are set; without either role,
    // a rendered explicit `crew` may select the modern flat assignment.
    let role_override = role_overridden_spec(t, ctx, &rendered_input)?;
    match (&t.spec, &t.session) {
        (ActivityV2Spec::AgentLoop(inline_spec), Some(binding)) => {
            let agent_spec_owned = role_override.clone();
            let agent_spec = agent_spec_owned.as_ref().unwrap_or(inline_spec);
            // Sessions only bind in HTTP mode; the loader rejects `loop +
            // session + cli` via `validate_job_loop_session_backends`, but we
            // also guard structurally here in case a flat (non-loop) target
            // declares session + cli.
            if agent_spec.backend != orbit_common::types::activity_job::Backend::Http {
                return Err(DispatchError::JobValidation(format!(
                    "step `{}`: `session:` binding requires backend: http (got {})",
                    step.id,
                    agent_spec.backend.as_str()
                )));
            }
            if !agent_spec.provider.has_http_transport() {
                return Err(DispatchError::UnwiredHttpTransport {
                    provider: agent_spec.provider.as_str().to_string(),
                });
            }
            // Reuse the named Session across calls. Held under a Mutex so
            // siblings could theoretically reference the same binding; the
            // validator rejects that shape, but the lock is cheap.
            let mut sessions = ctx.sessions.lock().expect("sessions poisoned");
            let entry = sessions.entry(binding.clone()).or_insert_with(|| {
                let model = agent_spec
                    .model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MODEL_FOR_SESSION.to_string());
                let provider = agent_spec.provider.as_str();
                Session::new(provider, model, &agent_spec.instruction, None)
            });
            let api_key = ctx.host.api_key_for("anthropic").ok();
            run_agent_loop_outcome(
                step,
                agent_spec,
                api_key.as_deref(),
                entry,
                &rendered_input,
                t.fs_profile.as_deref(),
                ctx,
            )
        }
        (ActivityV2Spec::AgentLoop(_inline_spec), None) => {
            // Route through the backend-aware dispatcher so a step with
            // `backend: cli` lands on the CLI runner rather than the HTTP
            // driver. When an effective role is present, swap the spec for
            // the resolver-overridden one before dispatch so the runner sees
            // `[agent.<role>]` values instead of inline ones.
            let dispatched_spec_storage = role_override
                .as_ref()
                .map(|spec| ActivityV2Spec::AgentLoop(spec.clone()));
            let dispatched_spec = dispatched_spec_storage.as_ref().unwrap_or(&t.spec);
            let dispatch = dispatch_v2_activity(V2DispatchInput {
                activity_name: &step.id,
                spec: dispatched_spec,
                fs_profile: t.fs_profile.as_deref(),
                input: rendered_input.clone(),
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            persist_dispatch_invocation(ctx, &step.id, &rendered_input, &dispatch);
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                message: dispatch.message,
                skipped: false,
            })
        }
        _ => {
            // Deterministic / Shell dispatch via the existing Phase 2 entry.
            let dispatch = dispatch_v2_activity(V2DispatchInput {
                activity_name: &step.id,
                spec: &t.spec,
                fs_profile: t.fs_profile.as_deref(),
                input: rendered_input.clone(),
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            persist_dispatch_invocation(ctx, &step.id, &rendered_input, &dispatch);
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                message: dispatch.message,
                skipped: false,
            })
        }
    }
}

/// Persist the invocation trace for a dispatched step.
///
/// [ORB-10367] **Non-fatal by contract.** This is telemetry: a failed write
/// (schema drift, a locked or unwritable database, a full disk) must never
/// discard agent work that already completed. The failure is logged loudly
/// and surfaced on the run record as `telemetry.persist_failed`; the step's
/// success stays decided solely by its own work.
pub(super) fn persist_dispatch_invocation(
    ctx: &ExecCtx<'_>,
    step_id: &str,
    input: &Value,
    dispatch: &super::super::dispatcher::DispatchOutcome,
) {
    let Some(invocation) = dispatch.invocation.as_ref() else {
        return;
    };

    if let Err(error) = ctx.host.persist_invocation_trace(
        &ctx.run_id,
        step_id,
        &invocation.provider,
        invocation.model.as_deref(),
        input,
        &invocation.trace,
    ) {
        ctx.audit
            .note_telemetry_failure("invocation_trace", Some(step_id), &error);
    }
}

pub(super) fn run_agent_loop_outcome(
    step: &JobV2Step,
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    session: &mut Session,
    input: &Value,
    fs_profile: Option<&str>,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let started = Instant::now();
    let outcome = drive_agent_loop_with_session(
        spec,
        api_key,
        &ctx.run_id,
        ctx.audit.clone(),
        session,
        input,
        ctx.host,
        fs_profile,
    )?;
    let trace = super::super::dispatcher::loop_outcome_trace(
        &outcome,
        started.elapsed().as_millis() as u64,
    );
    // [ORB-10367] Telemetry, not correctness: the agent loop already ran to
    // completion, so a failed trace write is recorded, never propagated.
    if let Err(error) = ctx.host.persist_invocation_trace(
        &ctx.run_id,
        &step.id,
        spec.provider.as_str(),
        spec.model.as_deref(),
        input,
        &trace,
    ) {
        ctx.audit
            .note_telemetry_failure("invocation_trace", Some(&step.id), &error);
    }
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "final_message".to_string(),
        Value::String(outcome.final_message.clone()),
    );
    metadata.insert(
        "terminate_reason".to_string(),
        Value::String(format!("{:?}", outcome.terminate_reason)),
    );
    let out_json = super::super::dispatcher::agent_loop_output_from_final_message(
        &outcome.final_message,
        metadata,
    );
    record_pipeline(ctx, &step.id, out_json.clone());
    Ok(StepOutcome {
        success: true,
        output: out_json,
        message: None,
        skipped: false,
    })
}

#[cfg(test)]
pub(super) fn replay_active() -> bool {
    std::env::var("ORBIT_V2_REPLAY").is_ok() || std::env::var("ORBIT_V2_REPLAY_FIXTURE").is_ok()
}

/// Build a crew-overridden clone of an [`AgentLoopSpec`]. A declared activity
/// role uses the role-labelled path; otherwise an explicit rendered `crew`
/// input may select the modern flat crew assignment without inventing a role.
pub(super) fn role_overridden_spec(
    t: &TargetStep,
    ctx: &ExecCtx<'_>,
    rendered_input: &Value,
) -> Result<Option<AgentLoopSpec>, DispatchError> {
    let ActivityV2Spec::AgentLoop(inline_spec) = &t.spec else {
        return Ok(None);
    };
    let resolved = match t.role.or(inline_spec.role) {
        Some(effective_role) => {
            resolve_agent_settings(effective_role, ctx.host, inline_spec, &ctx.input)
        }
        None => {
            let Some(resolved) =
                resolve_explicit_crew_settings(ctx.host, inline_spec, rendered_input)?
            else {
                return Ok(None);
            };
            resolved
        }
    };
    let mut spec = inline_spec.clone();
    apply_resolved_settings(&mut spec, &resolved);
    Ok(Some(spec))
}

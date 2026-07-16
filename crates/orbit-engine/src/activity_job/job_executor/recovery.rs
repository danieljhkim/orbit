use super::*;

pub(super) fn recover_or_return_original(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
    original_err: DispatchError,
    attempt: u32,
    max_attempts: u32,
) -> Result<StepOutcome, DispatchError> {
    let Some(recovery) = recovery_activity_for_step(step, ctx) else {
        return Err(original_err);
    };

    // L-0086: post-recovery re-entry is bounded and reports the resumed outcome.
    if attempt_recovery_activity(step, ctx, &recovery, &original_err, attempt, max_attempts) {
        let resumed_attempt = attempt.saturating_add(1);
        return match run_step_body(step, ctx) {
            Ok(outcome) if outcome.success => {
                emit_recovery_resumed(step, ctx, resumed_attempt, "success", None);
                Ok(outcome)
            }
            Ok(outcome) => {
                let message = outcome.message.unwrap_or_else(|| {
                    format!(
                        "post-recovery attempt {resumed_attempt} of step `{}` returned an unsuccessful outcome",
                        step.id
                    )
                });
                emit_recovery_resumed(step, ctx, resumed_attempt, "failed", Some(message.clone()));
                Err(DispatchError::JobExecution(message))
            }
            Err(err) => {
                emit_recovery_resumed(step, ctx, resumed_attempt, "error", Some(err.to_string()));
                Err(err)
            }
        };
    }

    Err(original_err)
}

fn emit_recovery_resumed(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
    attempt: u32,
    outcome: &str,
    error_message: Option<String>,
) {
    emit_job_event_lossy(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepRecoveryResumed {
            step_id: step.id.clone(),
            attempt,
            outcome: outcome.to_string(),
            error_message,
        },
    );
}

pub(super) fn recovery_activity_for_step(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
) -> Option<ResolvedRecoveryActivity> {
    match (
        step.recovery_activity.as_ref(),
        step.resolved_recovery_activity.as_ref(),
    ) {
        (Some(name), Some(activity)) => Some(ResolvedRecoveryActivity {
            name: name.clone(),
            spec: activity.spec.clone(),
        }),
        (Some(_), None) => None,
        _ => ctx.recovery_activity.clone(),
    }
}

pub(super) fn attempt_recovery_activity(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
    recovery: &ResolvedRecoveryActivity,
    original_err: &DispatchError,
    attempt: u32,
    max_attempts: u32,
) -> bool {
    let input = serde_json::json!({
        "failed_step_id": step.id,
        "activity_name": step_activity_name(step),
        "error_message": original_err.to_string(),
        "attempt": attempt,
        "max_attempts": max_attempts,
    });
    let role_overridden_spec = role_overridden_recovery_spec(recovery, ctx);
    let spec = role_overridden_spec.as_ref().unwrap_or(&recovery.spec);
    let dispatch = dispatch_v2_activity_without_run_id_injection(V2DispatchInput {
        activity_name: &recovery.name,
        spec,
        fs_profile: step_fs_profile(step),
        input: input.clone(),
        audit: ctx.audit.clone(),
        run_id: &ctx.run_id,
        host: Some(ctx.host),
    });

    let recovery_succeeded = match dispatch {
        Ok(dispatch) if dispatch.success => {
            // [ORB-00414] Best-effort dispatch-invocation persistence (a DB
            // record, not an audit-envelope write); a failure here is
            // intentionally non-fatal and does not affect recovery outcome. The
            // audit trail of the recovery attempt is emitted separately below.
            let _ = persist_dispatch_invocation(ctx, &recovery.name, &input, &dispatch);
            true
        }
        Ok(dispatch) => {
            // [ORB-00414] See above: non-audit DB persistence, non-fatal.
            let _ = persist_dispatch_invocation(ctx, &recovery.name, &input, &dispatch);
            false
        }
        Err(_) => false,
    };

    emit_job_event_lossy(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepRecoveryAttempted {
            step_id: step.id.clone(),
            recovery_activity: recovery.name.clone(),
            recovery_succeeded,
        },
    );

    recovery_succeeded
}

pub(super) fn role_overridden_recovery_spec(
    recovery: &ResolvedRecoveryActivity,
    ctx: &ExecCtx<'_>,
) -> Option<ActivityV2Spec> {
    let ActivityV2Spec::AgentLoop(inline_spec) = &recovery.spec else {
        return None;
    };
    let role = inline_spec.role?;
    let resolved = resolve_agent_settings(role, ctx.host, inline_spec, &ctx.input);
    let mut spec = inline_spec.clone();
    apply_resolved_settings(&mut spec, &resolved);
    Some(ActivityV2Spec::AgentLoop(spec))
}

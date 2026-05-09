use super::*;

pub fn validate_job(job: &JobV2) -> Result<(), DispatchError> {
    if let Some(name) = &job.recovery_activity
        && job.resolved_recovery_activity.is_none()
    {
        return Err(DispatchError::JobValidation(format!(
            "job recovery_activity `{name}` was not resolved — caller must run \
             `resolve_job_catalog_refs_for_execution` at load time before dispatch"
        )));
    }
    for step in &job.steps {
        validate_step(step)?;
    }
    Ok(())
}

pub(super) fn validate_step(step: &JobV2Step) -> Result<(), DispatchError> {
    if let Some(name) = &step.recovery_activity
        && step.resolved_recovery_activity.is_none()
    {
        return Err(DispatchError::JobValidation(format!(
            "step `{}` recovery_activity `{name}` was not resolved — caller must run \
             `resolve_job_catalog_refs_for_execution` at load time before dispatch",
            step.id
        )));
    }

    match &step.body {
        JobV2StepBody::Parallel { parallel } => {
            let mut seen: HashMap<&str, &str> = HashMap::new();
            for branch in &parallel.branches {
                collect_session_bindings(branch, &mut seen, &step.id)?;
                validate_step(branch)?;
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            // Workers execute concurrently against one another. If the worker
            // template names a session binding, every worker would share it.
            let mut seen: HashMap<&str, &str> = HashMap::new();
            collect_session_bindings(&fan_out.worker, &mut seen, &step.id)?;
            if !seen.is_empty() {
                return Err(DispatchError::JobValidation(format!(
                    "fan_out step `{}` worker names session binding(s); workers run concurrently and Session is !Sync",
                    step.id
                )));
            }
            validate_step(&fan_out.worker)?;
        }
        JobV2StepBody::Loop { loop_ } => {
            for body in &loop_.steps {
                validate_step(body)?;
            }
        }
        JobV2StepBody::Target(_) => {}
        JobV2StepBody::TargetRef(_) => {
            // Surfaces as a structural error in `run_step_body`; no session
            // binding to validate here.
        }
    }
    Ok(())
}

pub(super) fn collect_session_bindings<'a>(
    step: &'a JobV2Step,
    seen: &mut HashMap<&'a str, &'a str>,
    parent_id: &'a str,
) -> Result<(), DispatchError> {
    match &step.body {
        JobV2StepBody::Target(t) => {
            if let Some(binding) = &t.session {
                let name: &str = binding.as_str();
                if let Some(other) = seen.insert(name, step.id.as_str()) {
                    return Err(DispatchError::JobValidation(format!(
                        "parallel siblings under `{parent_id}` both bind session `{name}`: steps `{other}` and `{}`",
                        step.id
                    )));
                }
            }
        }
        JobV2StepBody::Parallel { parallel } => {
            for b in &parallel.branches {
                collect_session_bindings(b, seen, parent_id)?;
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            collect_session_bindings(&fan_out.worker, seen, parent_id)?;
        }
        JobV2StepBody::Loop { loop_ } => {
            for b in &loop_.steps {
                collect_session_bindings(b, seen, parent_id)?;
            }
        }
        JobV2StepBody::TargetRef(_) => {
            // Can't collect bindings from an unresolved ref; dispatcher
            // surfaces the structural error.
        }
    }
    Ok(())
}

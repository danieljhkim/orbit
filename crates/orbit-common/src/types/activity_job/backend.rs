//! v2 execution-backend resolution and §3.2 HTTP-only feature enforcement.
//!
//! `Backend::Auto` is resolved at load time per §3.1 precedence; callers
//! (orbit-core entry points) compute the concrete backend from flag → env →
//! config → default and then call [`resolve_activity_backends`] or
//! [`resolve_job_backends`] on the parsed asset to rewrite every `Auto` in
//! place to the resolved concrete value.
//!
//! [`validate_job_loop_session_backends`] enforces §3.2 item 1: a step inside
//! a `loop:` that declares a `session:` binding must resolve to `backend:
//! http`. `auto`-resolved-to-cli steps are rejected identically — resolution
//! has already run, so this check operates on concrete backends only.

use thiserror::Error;

use super::activity_v2::{ActivityV2, ActivityV2Spec, Backend};
use super::job_v2::{JobV2, JobV2Step, JobV2StepBody, LoopBlock};

/// §3.2 HTTP-only feature list — the item numbers are part of the public
/// error message per the task AC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpOnlyFeature {
    /// §3.2 item 1 — cross-iteration `session:` binding.
    CrossIterationSession,
}

impl HttpOnlyFeature {
    pub fn item_number(self) -> u32 {
        match self {
            HttpOnlyFeature::CrossIterationSession => 1,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            HttpOnlyFeature::CrossIterationSession => "cross-iteration `session:` binding",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BackendConstraintError {
    #[error(
        "asset_load: {asset_path} — step `{step_id}` declares `session: {session_name}` inside `loop:` with backend: cli. {feature_desc} is an HTTP-only feature (§3.2 item {item_number}). Fix: set `backend: http`, or remove the `session:` binding and accept cold-start cost per iteration."
    )]
    LoopSessionOnCli {
        asset_path: String,
        step_id: String,
        session_name: String,
        item_number: u32,
        feature_desc: &'static str,
    },
}

/// Walk a single activity asset and rewrite `Backend::Auto` to the resolved
/// concrete backend. No-op when the activity isn't `agent_loop` or the backend
/// is already concrete.
pub fn resolve_activity_backends(activity: &mut ActivityV2, resolved: Backend) {
    if let ActivityV2Spec::AgentLoop(spec) = &mut activity.spec
        && spec.backend == Backend::Auto
    {
        spec.backend = resolved;
    }
}

/// Walk a job and rewrite every `Backend::Auto` buried inside nested steps
/// (including parallel branches, fan-out workers, and loop bodies) to the
/// resolved concrete backend.
pub fn resolve_job_backends(job: &mut JobV2, resolved: Backend) {
    if let Some(activity) = &mut job.resolved_recovery_activity {
        resolve_activity_backends(activity, resolved);
    }
    for step in &mut job.steps {
        resolve_step_backends(step, resolved);
    }
}

fn resolve_step_backends(step: &mut JobV2Step, resolved: Backend) {
    match &mut step.body {
        JobV2StepBody::Target(target) => {
            if let ActivityV2Spec::AgentLoop(spec) = &mut target.spec
                && spec.backend == Backend::Auto
            {
                spec.backend = resolved;
            }
        }
        JobV2StepBody::TargetRef(_) => {
            // A surviving TargetRef means `resolve_job_target_refs` wasn't
            // run first. Leave it alone here; the dispatcher surfaces the
            // structural error.
        }
        JobV2StepBody::Parallel { parallel } => {
            for branch in &mut parallel.branches {
                resolve_step_backends(branch, resolved);
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            resolve_step_backends(&mut fan_out.worker, resolved);
        }
        JobV2StepBody::Loop { loop_ } => {
            for nested in &mut loop_.steps {
                resolve_step_backends(nested, resolved);
            }
        }
    }
}

/// Enforce §3.2 item 1: any step inside a `loop:` body with `session:`
/// declared must resolve to `backend: http`. Assumes `resolve_job_backends`
/// has already run so every `Auto` is concrete.
pub fn validate_job_loop_session_backends(
    job: &JobV2,
    asset_path: &str,
) -> Result<(), BackendConstraintError> {
    for step in &job.steps {
        validate_step(step, asset_path, false)?;
    }
    Ok(())
}

fn validate_step(
    step: &JobV2Step,
    asset_path: &str,
    inside_loop: bool,
) -> Result<(), BackendConstraintError> {
    match &step.body {
        JobV2StepBody::Target(target) => {
            if inside_loop
                && let (Some(session), ActivityV2Spec::AgentLoop(spec)) =
                    (&target.session, &target.spec)
                && spec.backend == Backend::Cli
            {
                let feature = HttpOnlyFeature::CrossIterationSession;
                return Err(BackendConstraintError::LoopSessionOnCli {
                    asset_path: asset_path.to_string(),
                    step_id: step.id.clone(),
                    session_name: session.clone(),
                    item_number: feature.item_number(),
                    feature_desc: feature.description(),
                });
            }
            Ok(())
        }
        JobV2StepBody::TargetRef(_) => {
            // See `resolve_step_backends` — a surviving ref is structural
            // breakage, not a §3.2 concern. Skip silently; the dispatcher's
            // `UnresolvedTargetRef` surfaces it.
            Ok(())
        }
        JobV2StepBody::Parallel { parallel } => {
            for branch in &parallel.branches {
                validate_step(branch, asset_path, inside_loop)?;
            }
            Ok(())
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            validate_step(&fan_out.worker, asset_path, inside_loop)
        }
        JobV2StepBody::Loop { loop_ } => validate_loop_block(loop_, asset_path),
    }
}

fn validate_loop_block(block: &LoopBlock, asset_path: &str) -> Result<(), BackendConstraintError> {
    for step in &block.steps {
        validate_step(step, asset_path, true)?;
    }
    Ok(())
}

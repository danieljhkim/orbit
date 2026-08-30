use crate::workspace::Workspace;

use super::WorkflowError;

/// Pipeline mode for shipping work: open a PR or apply locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipMode {
    Pr,
    Local,
}

impl ShipMode {
    pub fn as_input_value(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Local => "local",
        }
    }

    pub fn parse(value: &str) -> Result<Self, WorkflowError> {
        match value {
            "pr" => Ok(Self::Pr),
            "local" => Ok(Self::Local),
            other => Err(WorkflowError::Invalid(format!(
                "unknown ship mode '{other}' (expected 'pr' or 'local')"
            ))),
        }
    }
}

/// Resolve a workspace's effective ship mode, defaulting safely to PR mode.
pub fn resolved_ship_mode(workspace: &Workspace) -> ShipMode {
    workspace
        .ship_mode
        .as_deref()
        .and_then(|value| ShipMode::parse(value).ok())
        .unwrap_or(ShipMode::Pr)
}

/// The per-task pipeline a gate run dispatches into.
///
/// `task_gate_pipeline` names its child job `task_{{ input.mode }}_pipeline`,
/// so this enum's input value *is* the routing decision. [`ShipMode`] stays the
/// operator-facing run-level choice (`pr` / `local`); this is the per-task
/// refinement of it, which is why the two are separate types rather than one
/// widened enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineMode {
    /// `task_pr_pipeline` — commit, push, open a pull request, promote.
    Pr,
    /// `task_local_pipeline` — commit and promote without publishing.
    Local,
    /// `task_ci_remediation_pipeline` — host-owned CI discovery before the
    /// agent runs and host-owned candidate verification after publication.
    CiRemediation,
}

impl PipelineMode {
    pub fn as_input_value(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Local => "local",
            Self::CiRemediation => "ci_remediation",
        }
    }

    pub fn parse(value: &str) -> Result<Self, WorkflowError> {
        match value {
            "pr" => Ok(Self::Pr),
            "local" => Ok(Self::Local),
            "ci_remediation" => Ok(Self::CiRemediation),
            other => Err(WorkflowError::Invalid(format!(
                "unknown pipeline mode '{other}' (expected 'pr', 'local', or 'ci_remediation')"
            ))),
        }
    }

    pub fn from_ship_mode(mode: ShipMode) -> Self {
        match mode {
            ShipMode::Pr => Self::Pr,
            ShipMode::Local => Self::Local,
        }
    }
}

/// Choose the pipeline for one task, given the run's requested default mode.
///
/// The CI-remediation override applies only on top of `pr`: that pipeline
/// publishes a candidate commit and then verifies it on GitHub Actions, so it
/// is a refinement of publishing delivery and has nothing to refine when the
/// caller asked for local-only delivery. A `local` run therefore stays exactly
/// as it was, and so does any `pr` run whose task is not CI-shaped.
pub fn pipeline_mode_for_task<S: AsRef<str>>(
    default_mode: PipelineMode,
    tags: &[S],
) -> PipelineMode {
    if default_mode == PipelineMode::Pr
        && tags
            .iter()
            .any(|tag| tag.as_ref() == crate::task::CI_FAILURE_REMEDIATION_TAG)
    {
        return PipelineMode::CiRemediation;
    }
    default_mode
}

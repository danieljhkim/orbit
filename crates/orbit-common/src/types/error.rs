use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOriginMode {
    Local,
    Federated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOrigin {
    pub mode: ArtifactOriginMode,
    pub worktree_root: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotFoundKind {
    Tool,
    Task,
    Skill,
    Job,
    JobRun,
    Activity,
    Adr,
    DesignFeature,
    AgentSession,
    Workspace,
}

impl std::fmt::Display for NotFoundKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Tool => "tool",
            Self::Task => "task",
            Self::Skill => "skill",
            Self::Job => "job",
            Self::JobRun => "job run",
            Self::Activity => "activity",
            Self::Adr => "ADR",
            Self::DesignFeature => "design feature",
            Self::AgentSession => "agent session",
            Self::Workspace => "workspace",
        };
        f.write_str(kind)
    }
}

/// Evidence behind [`OrbitError::DependencyNotDelivered`]: which task was
/// refused, which of its done dependencies is missing from the base, the base
/// itself, and `detail` — the delivery commits found elsewhere in the
/// repository plus the remedy.
///
/// Boxed into the error enum: five inline strings would widen every
/// `Result<_, OrbitError>` in the workspace past the large-error threshold for
/// one rare refusal.
#[derive(Debug, Serialize)]
pub struct DependencyNotDelivered {
    pub task_id: String,
    pub dependency_id: String,
    pub base_ref: String,
    pub base_sha: String,
    pub detail: String,
}

/// Evidence behind [`OrbitError::WorkspaceClaimHeld`]: the refused operation,
/// the incumbent holder, its claim id, and the instant the claim lapses.
///
/// The holder's token is deliberately absent. A refusal travels to the
/// contender, and a contender that could read the token out of its own refusal
/// would turn the claim into a formality.
///
/// Boxed into the error enum for the same reason as
/// [`DependencyNotDelivered`]: four inline strings for one rare refusal would
/// widen every `Result<_, OrbitError>` in the workspace.
#[derive(Debug, Serialize)]
pub struct WorkspaceClaimHeld {
    /// The governed operation that was refused, e.g. `orbit.workflow.ship`.
    pub operation: String,
    /// The incumbent's actor label.
    pub holder: String,
    pub claim_id: String,
    /// RFC 3339 instant at which the claim stops blocking on its own.
    pub expires_at: String,
}

#[derive(Debug, Error, Serialize)]
#[non_exhaustive]
/// Keep this widely propagated error below its 128-byte size budget. Box the
/// payload of any future multi-field variant so adding it does not widen every
/// `Result<_, OrbitError>` in the workspace.
pub enum OrbitError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("{kind} not found: {id}")]
    NotFound { kind: NotFoundKind, id: String },
    /// A governed operation was refused because the caller lacked the required
    /// capability [ORB-10453]. Distinct from [`Self::PolicyDenied`], which is
    /// `orbit-policy`'s filesystem-scoping refusal: this one is about *who is
    /// asking*, not *which path was touched*, and every surface maps it to a
    /// `denied` audit status so refusals stay separable from failures.
    #[error("capability denied: {0}")]
    CapabilityDenied(String),
    #[error("Invalid ADR status transition: {0}")]
    AdrInvalidTransition(String),
    #[error("{kind} artifact unavailable for {id}")]
    RemoteArtifactUnavailable {
        kind: NotFoundKind,
        id: String,
        artifact_origin: ArtifactOrigin,
    },
    #[error("{kind} artifact is not local to the current worktree: {id}")]
    ArtifactNotLocal {
        kind: NotFoundKind,
        id: String,
        artifact_origin: ArtifactOrigin,
    },
    #[error("companion not installed: {0}")]
    CompanionNotInstalled(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("sensitive input rejected for `{field}`: {reason}")]
    SensitiveInput { field: String, reason: String },
    #[error("invalid input: {message}")]
    InvalidInputDiagnostic {
        message: String,
        did_you_mean: Vec<String>,
    },
    #[error("skill validation failed: {0}")]
    SkillValidation(String),
    #[error("job validation failed: {0}")]
    JobValidation(String),
    #[error("agent protocol violation: {0}")]
    AgentProtocolViolation(String),
    #[error("unsupported agent provider: {0}")]
    UnsupportedAgentProvider(String),
    #[error("owner unavailable: {0}")]
    OwnerUnavailable(String),
    #[error("owner negotiation failed: {0}")]
    OwnerNegotiation(String),
    #[error("owner call outcome unknown for {mcp_call_id}: {message}")]
    OutcomeUnknown {
        mcp_call_id: String,
        message: String,
    },
    #[error("remote tool failed ({code}): {message}")]
    RemoteTool {
        code: String,
        message: String,
        payload: serde_json::Value,
    },
    #[error("execution failed: {0}")]
    Execution(String),
    #[error(
        "run cancellation incomplete: pid={pid}, pgid={pgid:?}, term_sent={term_sent}, kill_sent={kill_sent}, leader_alive={leader_alive}, group_alive={group_alive}"
    )]
    RunCancellationIncomplete {
        pid: u32,
        pgid: Option<i32>,
        term_sent: bool,
        kill_sent: bool,
        leader_alive: bool,
        group_alive: bool,
    },
    #[error("task bundle corrupt for {task_id} at {path}: {reason}")]
    TaskBundleCorrupt {
        task_id: String,
        path: String,
        reason: String,
    },
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid task status transition: {0}")]
    TaskStatusTransition(String),
    /// A workflow run was refused because a dependency that reached `done` has
    /// not been delivered into the base the run would be cut from
    /// [ORB-10464]. Distinct from [`Self::TaskStatusTransition`]: the
    /// lifecycle transition is legal, the *baseline* is wrong.
    #[error(
        "task '{}' depends on '{}', which is done but not delivered into base '{}' ({}): {}",
        .0.task_id, .0.dependency_id, .0.base_ref, .0.base_sha, .0.detail
    )]
    DependencyNotDelivered(Box<DependencyNotDelivered>),
    /// A ship submission naming explicit task ids was refused because one of
    /// them is already carried by a non-terminal run [ORB-10544]. Raised by the
    /// shared submission path, so every dispatch surface refuses the duplicate
    /// identically; the payload names the contended task and the run that holds
    /// it so a caller can wait on or cancel that run rather than re-dispatch.
    #[error(
        "task {task_id} already has an in-flight run ({run_id}); wait for it to finish or cancel it"
    )]
    ShipRunInFlight { task_id: String, run_id: String },
    /// A governed workflow operation was refused because another operator holds
    /// the exclusive workspace claim [ADR-0352, ORB-10709]. Raised by the shared
    /// run-submission path, so the refusal is identical on every surface, and
    /// the payload names the incumbent and the expiry instant so a caller can
    /// wait it out, ask the holder, or force-release rather than retry blindly.
    /// Contention rejects: never a silent queue, never a silent steal.
    #[error(
        "workspace claim is held by {} until {} ({}); {} requires that claim's token — present it as `claim_token`, wait for the claim to expire, or force-release it",
        .0.holder, .0.expires_at, .0.claim_id, .0.operation
    )]
    WorkspaceClaimHeld(Box<WorkspaceClaimHeld>),
    #[error("invalid job run state transition: {0}")]
    JobRunStateTransition(String),
    #[error("workspace error: {0}")]
    WorkspaceError(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("schema migration failed: {0}")]
    Migration(String),
}

const _: () = assert!(
    std::mem::size_of::<OrbitError>() <= 128,
    "OrbitError exceeds its 128-byte size budget; box multi-field variant payloads"
);

impl OrbitError {
    pub fn not_found(kind: NotFoundKind, id: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            id: id.into(),
        }
    }

    pub fn invalid_input_with_suggestions(
        message: impl Into<String>,
        did_you_mean: Vec<String>,
    ) -> Self {
        if did_you_mean.is_empty() {
            Self::InvalidInput(message.into())
        } else {
            Self::InvalidInputDiagnostic {
                message: message.into(),
                did_you_mean,
            }
        }
    }

    pub fn remote_artifact_unavailable(
        kind: NotFoundKind,
        id: impl Into<String>,
        artifact_origin: ArtifactOrigin,
    ) -> Self {
        Self::RemoteArtifactUnavailable {
            kind,
            id: id.into(),
            artifact_origin,
        }
    }

    pub fn artifact_not_local(
        kind: NotFoundKind,
        id: impl Into<String>,
        artifact_origin: ArtifactOrigin,
    ) -> Self {
        Self::ArtifactNotLocal {
            kind,
            id: id.into(),
            artifact_origin,
        }
    }

    pub fn artifact_origin(&self) -> Option<&ArtifactOrigin> {
        match self {
            Self::RemoteArtifactUnavailable {
                artifact_origin, ..
            }
            | Self::ArtifactNotLocal {
                artifact_origin, ..
            } => Some(artifact_origin),
            _ => None,
        }
    }

    pub fn did_you_mean(&self) -> Option<&[String]> {
        match self {
            Self::InvalidInputDiagnostic { did_you_mean, .. } if !did_you_mean.is_empty() => {
                Some(did_you_mean)
            }
            _ => None,
        }
    }

    /// The contended `(task_id, run_id)` of a ship duplicate-dispatch refusal,
    /// so projections (HTTP 409 body, MCP structured error) can name both
    /// without re-parsing the message.
    pub fn ship_run_in_flight(&self) -> Option<(&str, &str)> {
        match self {
            Self::ShipRunInFlight { task_id, run_id } => Some((task_id, run_id)),
            _ => None,
        }
    }

    /// The refused operation, incumbent holder, claim id, and expiry of a
    /// workspace-claim refusal, so projections (HTTP 409 body, MCP structured
    /// error) can name them without re-parsing the message [ORB-10709].
    pub fn workspace_claim_held(&self) -> Option<&WorkspaceClaimHeld> {
        match self {
            Self::WorkspaceClaimHeld(claim) => Some(claim),
            _ => None,
        }
    }

    pub fn task_bundle_corruption(&self) -> Option<(&str, &str, &str)> {
        match self {
            Self::TaskBundleCorrupt {
                task_id,
                path,
                reason,
            } => Some((task_id, path, reason)),
            _ => None,
        }
    }
}

impl From<std::io::Error> for OrbitError {
    fn from(err: std::io::Error) -> Self {
        OrbitError::Io(err.to_string())
    }
}

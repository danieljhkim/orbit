//! [ORB-10364] Caller-role gate on the learning *authoring* surfaces.
//!
//! Policy since 2026-07-18: task executors file **frictions**; project
//! learnings are authored by the orchestrator or by a human. Until this module
//! the rule lived entirely in orchestrator-side prompt text, and three
//! different executors violated it in a single day's queue (friction
//! F2026-07-102: separate executor-authored entries and an update to L-0082).
//! Because the learning store feeds
//! scope injection and curation sweeps, every unreviewed executor-authored
//! entry costs real re-anchoring work downstream. Prompt text is not an
//! enforcement layer; this module is.
//!
//! **Role derivation.** The caller's role comes from the agent-identity env
//! pair the audit middleware already reads — `ORBIT_AGENT_NAME` /
//! `ORBIT_AGENT_MODEL`, assembled for every spawned run by `orbit-engine`'s
//! `provenance_env` builder — consumed here through
//! [`ActorIdentity::from_env`]. There is deliberately no second identity
//! mechanism to keep in sync.
//!
//! **What is gated.** The authoring surfaces named by the policy: `learning
//! add`, `learning update`, `learning supersede`, and `learning archive`
//! [ORB-10469], in both their CLI and `orbit.learning.*` tool forms. Reads
//! (`show`, `list`, `search`, `stats`) are untouched in every context, as are
//! `sync`, `prune`'s own bulk sweep, and the store-level writers used by the
//! dashboard's human-driven API and test fixtures.
//!
//! **Nothing is at stake on refusal.** [ORB-10725] removed the multi-host
//! preallocate-then-finalize path: learning IDs are allocated per workspace by
//! the owning machine (ADR-0357), so `orbit.learning.add` is one owner-local
//! transaction and this gate sits on the single authoring surface in front of
//! it. A refusal cannot strand an already-issued ID, because no ID exists
//! until the write itself.

use orbit_common::types::OrbitError;

use crate::context::{ActorIdentity, ActorKind};

/// Deliberate opt-in for an orchestrator that dispatches curation work *as* an
/// agent (ORB-10362 is exactly this shape). Set on the dispatch — never
/// inherited by accident, because the engine's provenance builder does not
/// emit it. Accepts the same truthy spellings as `ORBIT_MANAGED_RUN_CONTEXT`.
pub const LEARNING_AUTHOR_OPT_IN_ENV: &str = "ORBIT_LEARNING_AUTHOR";

/// Longest attempted body echoed back in a refusal before truncation. Long
/// enough to preserve a real observation verbatim, short enough that the error
/// stays readable on a terminal.
const ECHO_BODY_LIMIT: usize = 1500;

/// Who is attempting to author a learning, as derived from the process
/// environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearningAuthorRole {
    /// No agent-identity env pair: a human at a terminal, or a service (the
    /// dashboard) that carries its own request-derived attribution.
    Human,
    /// Agent context carrying the explicit [`LEARNING_AUTHOR_OPT_IN_ENV`]
    /// opt-in — the orchestrator's own curation dispatch.
    AuthorizedAgent { label: String },
    /// Agent context without the opt-in: a task executor.
    Executor { label: String },
}

/// The write a caller attempted, echoed back verbatim when it is refused so
/// the observation is not simply lost.
#[derive(Debug, Clone, Copy)]
pub enum LearningWriteAttempt<'a> {
    Add {
        summary: &'a str,
        body: &'a str,
    },
    Update {
        id: &'a str,
        summary: Option<&'a str>,
        body: Option<&'a str>,
    },
    Supersede {
        id: &'a str,
        with: &'a str,
    },
    /// [ORB-10469] Retire `id` without a replacement.
    Archive {
        id: &'a str,
    },
}

impl LearningWriteAttempt<'_> {
    fn operation(&self) -> &'static str {
        match self {
            Self::Add { .. } => "learning add",
            Self::Update { .. } => "learning update",
            Self::Supersede { .. } => "learning supersede",
            Self::Archive { .. } => "learning archive",
        }
    }

    /// The attempted content, rendered for the refusal message.
    fn echo(&self) -> String {
        match *self {
            Self::Add { summary, body } => {
                format!("summary: {summary}\nbody: {}", truncate_body(body))
            }
            Self::Update { id, summary, body } => {
                let mut echo = format!("id: {id}");
                if let Some(summary) = summary {
                    echo.push_str(&format!("\nsummary: {summary}"));
                }
                if let Some(body) = body {
                    echo.push_str(&format!("\nbody: {}", truncate_body(body)));
                }
                echo
            }
            Self::Supersede { id, with } => format!("id: {id}\nwith: {with}"),
            Self::Archive { id } => format!("id: {id}"),
        }
    }
}

/// Resolve the caller's authoring role from the ambient agent-identity env.
pub fn learning_author_role() -> LearningAuthorRole {
    let identity = ActorIdentity::from_env();
    match identity.kind {
        // Unknown is an attribution state, not a new authorization policy.
        // Preserve the pre-existing unenveloped CLI behavior until actor-aware
        // gating is designed and shipped separately.
        ActorKind::Unknown => LearningAuthorRole::Human,
        ActorKind::Human => LearningAuthorRole::Human,
        ActorKind::Agent if author_opt_in() => LearningAuthorRole::AuthorizedAgent {
            label: identity.label,
        },
        ActorKind::Agent => LearningAuthorRole::Executor {
            label: identity.label,
        },
    }
}

/// Refuse `attempt` when the caller is an executor-context agent.
///
/// Human contexts and the explicit orchestrator opt-in pass through. The
/// refusal is an [`OrbitError::PolicyDenied`] so the CLI audit middleware
/// records it as a denial rather than a failure.
pub fn ensure_learning_write_allowed(attempt: LearningWriteAttempt<'_>) -> Result<(), OrbitError> {
    match learning_author_role() {
        LearningAuthorRole::Human | LearningAuthorRole::AuthorizedAgent { .. } => Ok(()),
        LearningAuthorRole::Executor { label } => {
            Err(OrbitError::PolicyDenied(refusal(&label, &attempt)))
        }
    }
}

fn author_opt_in() -> bool {
    std::env::var(LEARNING_AUTHOR_OPT_IN_ENV)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"))
}

fn truncate_body(body: &str) -> String {
    if body.chars().count() <= ECHO_BODY_LIMIT {
        return body.to_string();
    }
    let head: String = body.chars().take(ECHO_BODY_LIMIT).collect();
    format!("{head}… [truncated]")
}

fn refusal(label: &str, attempt: &LearningWriteAttempt<'_>) -> String {
    format!(
        "`{operation}` is reserved for the orchestrator and human operators; this run is an \
agent executor (identity: {label}). Nothing was written.\n\n\
Executors record observations as friction instead:\n\
    orbit friction add --model <your agent family> --body \"<what happened and why it cost you>\"\n\
    (MCP/tool form: `orbit.friction.add`)\n\n\
The content you attempted is preserved here so the observation is not lost:\n\
{echo}\n\n\
If you are the orchestrator dispatching curation work as an agent, set \
{opt_in}=1 on that dispatch to opt in deliberately.",
        operation = attempt.operation(),
        echo = attempt.echo(),
        opt_in = LEARNING_AUTHOR_OPT_IN_ENV,
    )
}

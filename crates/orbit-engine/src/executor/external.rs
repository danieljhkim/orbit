//! `external` executor — the External Executor Protocol v1 transport.
//!
//! [`ExternalExecutor`] lets operators register a homegrown, out-of-process
//! executor by pointing an `executor_type: external` def at a binary or script
//! that speaks the protocol documented in
//! `docs/design/executors/specs/external-executor-protocol.md`. No recompile,
//! no linking, language-agnostic.
//!
//! Tier 1 reuses the `direct_agent` subprocess transport verbatim
//! ([`run_subprocess_executor`]): the request envelope is written to the
//! subprocess stdin, success/failure is signalled by the process exit code, and
//! the process runs unsandboxed (`NoSandbox`). Unlike `direct_agent` it carries
//! no agent-family `model_pair` semantics — an external def is just a command,
//! args, env, and an optional `model_flag`. See ADR-0196 / [ORB-00384].

use orbit_common::types::ExecutorDef;

use super::ActivityExecutor;
use super::direct_agent::run_subprocess_executor;
use crate::context::{AttemptOutcome, ExecutionContext, ExecutorHost};

/// Wire value of the `external` executor `spec_type`.
pub(crate) const EXTERNAL_SPEC_TYPE: &str = "external";

/// Generic out-of-process executor bound to an `executor_type: external` def.
pub struct ExternalExecutor {
    bound_executor: ExecutorDef,
}

impl ExternalExecutor {
    pub fn from_executor_def(def: ExecutorDef) -> Self {
        Self {
            bound_executor: def,
        }
    }
}

impl ActivityExecutor for ExternalExecutor {
    fn spec_type(&self) -> &str {
        EXTERNAL_SPEC_TYPE
    }

    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        run_subprocess_executor(&self.bound_executor, &host.agent(), execution)
    }
}

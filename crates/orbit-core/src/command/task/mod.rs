//! Task command/runtime helpers.

mod add;
mod helpers;
mod lint;
mod params;
mod paths;
mod query;
mod review;
mod transitions;
mod update;

pub use lint::{TaskLintFinding, TaskLintReport, TaskLintSeverity};
pub use params::{TaskAddParams, TaskUpdateParams};

pub(crate) use helpers::SYSTEM_ACTOR_LABEL;
pub(crate) use paths::{
    canonicalize_context_files_for_read, compute_task_add_warnings, context_workspace_root,
};
pub(crate) use transitions::{
    ensure_task_has_execution_plan, in_progress_transition_requires_plan,
};

#[cfg(test)]
mod tests;

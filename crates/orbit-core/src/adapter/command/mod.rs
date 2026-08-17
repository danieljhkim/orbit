//! Tool command/runtime helpers.
//!
//! Two independent concerns live behind this module and are re-exported here
//! so `command::tool::*` remains the single import path for consumers:
//! - [`dispatch`] — tool dispatch, audit correlation, agent-identity
//!   resolution, and the trusted MCP envelope boundary.
//! - [`registry`] — registry CRUD (list/show/add/remove/enable/disable/doctor).

mod dispatch;
mod registry;

#[cfg(test)]
mod tests;

pub use crate::runtime::tool_exec::DryRunResult;

pub use dispatch::{
    AuditContext, ToolDispatchOutcome, ToolEntryPoint, audit_role_label,
    audit_role_label_for_entry_point, execute_global_in_process_tool_dispatch,
    mark_tool_audit_recorded, take_tool_audit_recorded, trusted_mcp_audit_context,
};
pub use registry::{DoctorResult, DoctorStatus, ToolInfo};

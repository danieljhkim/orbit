//! Workspace session log [ADR-0363 / ORB-10784].

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitSessionLogAppendTool;
pub struct OrbitSessionLogListTool;
pub struct OrbitSessionLogResolveTool;

impl Tool for OrbitSessionLogAppendTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.session_log.append".to_string(),
            description: "Append a workspace session-log entry (status, note, or check_later). \
                 Bodies are immutable; check_later rows wake the drain scan until resolved."
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "kind".to_string(),
                    description: "Entry kind: status, note, or check_later.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Markdown body. Required, non-empty.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "related_task_ids".to_string(),
                    description: "Optional task ids this note refers to.".to_string(),
                    param_type: "string_list".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "related_run_ids".to_string(),
                    description: "Optional job-run ids this note refers to.".to_string(),
                    param_type: "string_list".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::SessionLogAppend)
    }
}

impl Tool for OrbitSessionLogListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.session_log.list".to_string(),
            description: "List workspace session-log entries. Filter by kind, unresolved \
                 check_later rows, or entries at/after `since` (RFC3339)."
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "kind".to_string(),
                    description: "Optional kind filter: status, note, or check_later.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "unresolved_only".to_string(),
                    description: "When true, return only unresolved check_later entries."
                        .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "since".to_string(),
                    description: "Optional RFC3339 lower bound on entry timestamps.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::SessionLogList)
    }
}

impl Tool for OrbitSessionLogResolveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.session_log.resolve".to_string(),
            description: "Mark a check_later session-log entry resolved. Other kinds cannot \
                 be resolved; bodies are never edited."
                .to_string(),
            parameters: vec![ToolParam {
                name: "id".to_string(),
                description: "Session-log id (SL-NNNN).".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::SessionLogResolve)
    }
}

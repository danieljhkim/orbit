use std::cell::Cell;
use std::path::Path;
use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, NotFoundKind, OrbitError, OrbitEvent, Role, StoredTool, ToolParam,
    ToolSessionContext, audit_execution_id, normalize_agent_family_for_model,
    normalize_optional_attribution_label,
};
use orbit_store::AuditEventInsertParams;
use orbit_tools::{ReservationOwnerContext, ToolContext};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::redact_sensitive_env_text;

pub use crate::runtime::pipeline::DryRunResult;

const ORBIT_MANAGED_RUN_CONTEXT_ENV: &str = "ORBIT_MANAGED_RUN_CONTEXT";

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    pub parameters: Vec<orbit_common::types::ToolParam>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct DoctorResult {
    pub tool_name: String,
    pub status: DoctorStatus,
    pub message: String,
}

/// Where a tool invocation arrived from. Captured in the audit row so a single
/// audit table can attribute tool calls back to their origin (CLI vs MCP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEntryPoint {
    /// `orbit tool run ...` from the CLI binary.
    Cli,
    /// MCP `tools/call` over `orbit mcp serve`.
    Mcp,
}

impl ToolEntryPoint {
    /// Subcommand value written into the audit row. Distinct values let
    /// downstream queries filter MCP-originated calls without a schema change.
    pub fn audit_subcommand(self) -> &'static str {
        match self {
            ToolEntryPoint::Cli => "run",
            ToolEntryPoint::Mcp => "run-mcp",
        }
    }
}

thread_local! {
    static TOOL_AUDIT_RECORDED: Cell<bool> = const { Cell::new(false) };
}

/// Mark that the runtime has already persisted an audit row for the current
/// tool invocation on this thread. Higher layers (the CLI `AuditGuard`) call
/// [`take_tool_audit_recorded`] during their own teardown to suppress a
/// duplicate emission. The signal is per-thread and one-shot.
pub fn mark_tool_audit_recorded() {
    TOOL_AUDIT_RECORDED.with(|cell| cell.set(true));
}

/// Read and clear the per-thread tool-audit-recorded flag set by
/// [`mark_tool_audit_recorded`]. Returns `true` if a runtime-level audit row
/// was emitted on this thread since the last call.
pub fn take_tool_audit_recorded() -> bool {
    TOOL_AUDIT_RECORDED.with(|cell| cell.replace(false))
}

/// Outcome of [`OrbitRuntime::execute_tool_command_dispatch`]: the tool's JSON
/// return value plus a flag indicating whether the runtime persisted an audit
/// row for this invocation.
#[derive(Debug)]
pub struct ToolDispatchOutcome {
    pub value: Value,
    pub audit_recorded: bool,
}

impl OrbitRuntime {
    /// Execute a tool by name and return its JSON value. CLI-callers use this
    /// path; the runtime tags the audit row with [`ToolEntryPoint::Cli`].
    pub fn execute_tool_command(
        &self,
        name: &str,
        input: Value,
        agent_override: Option<String>,
        model_override: Option<String>,
    ) -> Result<Value, OrbitError> {
        self.execute_tool_command_dispatch(
            name,
            input,
            agent_override,
            model_override,
            ToolEntryPoint::Cli,
        )
        .map(|outcome| outcome.value)
    }

    /// Execute a tool by name and return both the value and whether the
    /// runtime persisted an audit row. Callers that need to suppress a
    /// duplicate higher-level audit emission read `audit_recorded` (also
    /// available out-of-band via [`take_tool_audit_recorded`]).
    pub fn execute_tool_command_dispatch(
        &self,
        name: &str,
        input: Value,
        agent_override: Option<String>,
        model_override: Option<String>,
        entry_point: ToolEntryPoint,
    ) -> Result<ToolDispatchOutcome, OrbitError> {
        self.execute_tool_command_dispatch_with_session_context(
            name,
            input,
            agent_override,
            model_override,
            entry_point,
            ToolSessionContext::default(),
        )
    }

    pub fn execute_tool_command_dispatch_with_session_context(
        &self,
        name: &str,
        input: Value,
        agent_override: Option<String>,
        model_override: Option<String>,
        entry_point: ToolEntryPoint,
        session_context: ToolSessionContext,
    ) -> Result<ToolDispatchOutcome, OrbitError> {
        let start = Instant::now();
        let role_label =
            audit_role_label(&input, agent_override.as_deref(), model_override.as_deref());
        let working_directory = std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        let audit_context = resolve_audit_context(&input);

        // Closure boundary so any setup failure (e.g. an inconsistent
        // `agent`/`model` rejected by `resolve_agent_identity`) becomes a
        // recorded failure-status audit row rather than a silent early `?`
        // return. Without this, MCP-originated calls — which have no
        // surrounding `AuditGuard` to fall back on — would fail without any
        // audit row at all when identity setup is the cause.
        let result: Result<Value, OrbitError> = (|| {
            let allowed_tools = read_activity_tools_from_env();
            let (agent_name, model_name) = resolve_agent_identity(agent_override, model_override)?;
            let proc_allowed_programs = read_proc_allowed_programs_from_env();
            let cwd = std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
            let tool_context = ToolContext {
                cwd,
                session_context,
                allowed_tools,
                agent_name,
                model_name,
                workspace_root: None,
                proc_allowed_programs,
                reservation_owner: reservation_owner_from_env(),
                ..Default::default()
            };
            self.run_tool_with_context_and_role(name, input, Role::Admin, tool_context)
        })();
        let duration_ms = (start.elapsed().as_millis() as i64).max(1);

        let (status, exit_code, error_message) = match &result {
            Ok(_) => (AuditEventStatus::Success, 0, None),
            Err(OrbitError::PolicyDenied(msg)) => (
                AuditEventStatus::Denied,
                1,
                Some(redact_sensitive_env_text(msg)),
            ),
            Err(err) => (
                AuditEventStatus::Failure,
                1,
                Some(redact_sensitive_env_text(&err.to_string())),
            ),
        };

        let params = AuditEventInsertParams {
            execution_id: audit_execution_id("exec"),
            command: "tool".to_string(),
            subcommand: Some(entry_point.audit_subcommand().to_string()),
            tool_name: Some(name.to_string()),
            target_type: Some("tool".to_string()),
            target_id: Some(name.to_string()),
            role: role_label,
            status,
            exit_code,
            duration_ms,
            working_directory,
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message,
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
            task_id: audit_context.task_id,
            job_run_id: audit_context.job_run_id,
            activity_id: audit_context.activity_id,
            step_index: audit_context.step_index,
        };

        let audit_recorded = match self.record_audit_event(&params) {
            Ok(()) => {
                mark_tool_audit_recorded();
                true
            }
            Err(err) => {
                tracing::warn!("failed to persist tool audit event: {err}");
                false
            }
        };

        let value = result?;
        Ok(ToolDispatchOutcome {
            value,
            audit_recorded,
        })
    }
}

/// Audit-correlation context resolved at the tool-dispatch seam.
///
/// Each field follows the same precedence: caller-asserted value from the
/// tool input JSON (`task_id`, `job_run_id`, `activity_id`, `step_index`)
/// wins, falling back to the runtime-asserted env vars exported by the
/// engine when it spawned the agent (`ORBIT_TASK_ID`, `ORBIT_RUN_ID`,
/// `ORBIT_ACTIVITY_ID`, `ORBIT_STEP_INDEX`).
///
/// Treat input-supplied values as caller-asserted (an MCP client could lie).
/// Env-supplied values are the engine's ground truth.
#[derive(Debug, Default, Clone)]
struct AuditContext {
    task_id: Option<String>,
    job_run_id: Option<String>,
    activity_id: Option<String>,
    step_index: Option<i64>,
}

fn resolve_audit_context(input: &Value) -> AuditContext {
    fn input_str(input: &Value, key: &str) -> Option<String> {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }
    fn env_str(name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    AuditContext {
        task_id: input_str(input, "task_id").or_else(|| env_str("ORBIT_TASK_ID")),
        job_run_id: input_str(input, "job_run_id")
            .or_else(|| input_str(input, "run_id"))
            .or_else(|| env_str("ORBIT_RUN_ID")),
        activity_id: input_str(input, "activity_id").or_else(|| env_str("ORBIT_ACTIVITY_ID")),
        step_index: input
            .get("step_index")
            .and_then(Value::as_i64)
            .or_else(|| env_str("ORBIT_STEP_INDEX").and_then(|s| s.parse().ok())),
    }
}

fn reservation_owner_from_env() -> Option<ReservationOwnerContext> {
    let managed_context = std::env::var(ORBIT_MANAGED_RUN_CONTEXT_ENV)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    if !managed_context {
        return None;
    }

    std::env::var("ORBIT_RUN_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|owner_run_id| ReservationOwnerContext {
            owner_metadata_json: Some(
                serde_json::json!({
                    "source": "orbit_cli",
                })
                .to_string(),
            ),
            owner_run_id,
        })
}

fn read_agent_identity_from_env() -> (Option<String>, Option<String>) {
    let agent = std::env::var("ORBIT_AGENT_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let model = std::env::var("ORBIT_AGENT_MODEL")
        .ok()
        .filter(|s| !s.is_empty());
    (agent, model)
}

fn resolve_agent_identity(
    agent_override: Option<String>,
    model_override: Option<String>,
) -> Result<(Option<String>, Option<String>), OrbitError> {
    let (env_agent_name, env_model_name) = read_agent_identity_from_env();
    let has_override = agent_override
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || model_override
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let (agent, model) = if has_override {
        (agent_override, model_override)
    } else {
        (env_agent_name, env_model_name)
    };
    let agent = normalize_agent_family_for_model(agent.as_deref(), model.as_deref())?;
    // Tool-call identity crosses a trust boundary: agent-supplied `model`
    // strings are telemetry at best and may be aliases. Persist the canonical
    // family in the model slot for tool dispatch so comparisons never depend
    // on self-reported model text.
    Ok((agent.clone(), agent))
}

fn read_proc_allowed_programs_from_env() -> Vec<String> {
    std::env::var("ORBIT_PROC_ALLOWED_PROGRAMS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn read_activity_tools_from_env() -> Vec<String> {
    if std::env::var("ORBIT_TASK_ACTOR_KIND").ok().as_deref() != Some("agent") {
        return Vec::new();
    }
    std::env::var("ORBIT_ACTIVITY_TOOLS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the audit `role` label for a tool invocation.
///
/// Runtime envelope identity (`ORBIT_AGENT_*`) is authoritative for agent
/// activities and overwrites any self-reported `model` field in tool JSON.
/// Manual CLI/MCP calls without an envelope keep the legacy input/flag
/// precedence.
pub fn audit_role_label(
    input: &Value,
    agent_override: Option<&str>,
    model_override: Option<&str>,
) -> String {
    let (input_agent, input_model) = read_input_identity(input);
    let env_agent = std::env::var("ORBIT_AGENT_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let env_model = std::env::var("ORBIT_AGENT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let has_input_identity = input_agent.is_some() || input_model.is_some();
    let has_flag_identity = agent_override.is_some_and(|value| !value.trim().is_empty())
        || model_override.is_some_and(|value| !value.trim().is_empty());
    let has_env_identity = env_agent.is_some() || env_model.is_some();
    let (agent, model) = if has_env_identity && !has_flag_identity {
        let agent = normalize_agent_family_for_model(env_agent.as_deref(), env_model.as_deref())
            .ok()
            .flatten()
            .or(env_agent);
        (agent.clone(), agent)
    } else if has_input_identity {
        (input_agent, input_model)
    } else if has_flag_identity {
        (
            agent_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            model_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        )
    } else {
        (env_agent, env_model)
    };
    let agent = normalize_agent_family_for_model(agent.as_deref(), model.as_deref())
        .ok()
        .flatten()
        .or(agent);

    normalize_optional_attribution_label(model.as_deref().or(agent.as_deref()), model.as_deref())
        .unwrap_or_else(|| "agent".to_string())
}

fn read_input_identity(input: &Value) -> (Option<String>, Option<String>) {
    if let Value::Object(map) = input {
        let agent = map
            .get("agent")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let model = map
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        (agent, model)
    } else {
        (None, None)
    }
}

impl OrbitRuntime {
    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, OrbitError> {
        let registry_schemas = self.tool_registry().schemas();
        let stored_tools = self.stores().tools().list()?;

        let mut tools: Vec<ToolInfo> = registry_schemas
            .into_iter()
            .map(|schema| {
                let stored = stored_tools.iter().find(|s| s.name == schema.name);
                let enabled = stored.is_none_or(|s| s.enabled);
                ToolInfo {
                    name: schema.name.clone(),
                    description: schema.description.clone(),
                    enabled,
                    builtin: schema.builtin,
                    parameters: schema.parameters,
                }
            })
            .collect();

        // Add external tools that are in the store but not yet in the registry
        for stored in &stored_tools {
            if !stored.builtin && !tools.iter().any(|t| t.name == stored.name) {
                tools.push(ToolInfo {
                    name: stored.name.clone(),
                    description: stored.description.clone(),
                    enabled: stored.enabled,
                    builtin: false,
                    parameters: stored.parameters.clone(),
                });
            }
        }

        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    pub fn show_tool(&self, name: &str) -> Result<ToolInfo, OrbitError> {
        let schema = self
            .tool_registry()
            .get_schema(name)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))?;

        let stored = self.stores().tools().get(name)?;
        let enabled = stored.is_none_or(|s| s.enabled);

        Ok(ToolInfo {
            name: schema.name,
            description: schema.description,
            enabled,
            builtin: schema.builtin,
            parameters: schema.parameters,
        })
    }

    pub fn add_tool(
        &self,
        name: &str,
        path: &str,
        description: &str,
        parameters: Vec<ToolParam>,
    ) -> Result<(), OrbitError> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "path does not exist: {path}"
            )));
        }

        if let Some(schema) = self.tool_registry().get_schema(name)
            && schema.builtin
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot overwrite built-in tool '{name}'"
            )));
        }

        let tool = StoredTool {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            enabled: true,
            builtin: false,
            parameters,
        };

        self.with_mutation(|| {
            self.stores().tools().insert(&tool)?;
            Ok((
                (),
                OrbitEvent::ToolAdded {
                    name: name.to_string(),
                },
            ))
        })
    }

    pub fn remove_tool(&self, name: &str) -> Result<(), OrbitError> {
        if let Some(schema) = self.tool_registry().get_schema(name)
            && schema.builtin
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot remove built-in tool '{name}'; use 'orbit tool disable {name}' instead"
            )));
        }

        self.with_mutation(|| {
            let deleted = self.stores().tools().delete(name)?;
            if !deleted {
                return Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()));
            }
            Ok((
                (),
                OrbitEvent::ToolRemoved {
                    name: name.to_string(),
                },
            ))
        })
    }

    pub fn doctor(&self) -> Result<Vec<DoctorResult>, OrbitError> {
        let tools = self.list_tools()?;
        let mut results = Vec::new();

        for tool in &tools {
            if !tool.enabled {
                results.push(DoctorResult {
                    tool_name: tool.name.clone(),
                    status: DoctorStatus::Warning,
                    message: "tool is disabled".to_string(),
                });
                continue;
            }

            if tool.description.is_empty() {
                results.push(DoctorResult {
                    tool_name: tool.name.clone(),
                    status: DoctorStatus::Warning,
                    message: "missing description".to_string(),
                });
                continue;
            }

            if !tool.builtin
                && let Some(stored) = self.stores().tools().get(&tool.name)?
                && !stored.path.is_empty()
            {
                let path = std::path::Path::new(&stored.path);
                if !path.exists() {
                    results.push(DoctorResult {
                        tool_name: tool.name.clone(),
                        status: DoctorStatus::Error,
                        message: format!("executable not found: {}", stored.path),
                    });
                    continue;
                }
            }

            results.push(DoctorResult {
                tool_name: tool.name.clone(),
                status: DoctorStatus::Ok,
                message: String::new(),
            });
        }

        Ok(results)
    }

    pub fn enable_tool(&self, name: &str) -> Result<(), OrbitError> {
        self.set_tool_enabled_state(name, true)
    }

    pub fn disable_tool(&self, name: &str) -> Result<(), OrbitError> {
        self.set_tool_enabled_state(name, false)
    }

    fn set_tool_enabled_state(&self, name: &str, enabled: bool) -> Result<(), OrbitError> {
        if !self.tool_registry().has(name) {
            return Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()));
        }

        let existing = self.stores().tools().get(name)?;
        if existing.is_none() {
            let schema = self
                .tool_registry()
                .get_schema(name)
                .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))?;
            let tool = StoredTool {
                name: name.to_string(),
                path: String::new(),
                description: schema.description.clone(),
                enabled,
                builtin: schema.builtin,
                parameters: schema.parameters.clone(),
            };
            return self.with_mutation(|| {
                self.stores().tools().insert(&tool)?;
                let event = if enabled {
                    OrbitEvent::ToolEnabled {
                        name: name.to_string(),
                    }
                } else {
                    OrbitEvent::ToolDisabled {
                        name: name.to_string(),
                    }
                };
                Ok(((), event))
            });
        }

        self.with_mutation(|| {
            self.stores().tools().set_enabled(name, enabled)?;
            let event = if enabled {
                OrbitEvent::ToolEnabled {
                    name: name.to_string(),
                }
            } else {
                OrbitEvent::ToolDisabled {
                    name: name.to_string(),
                }
            };
            Ok(((), event))
        })
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Barrier, Mutex, MutexGuard, OnceLock};
    use std::thread;

    use serde_json::json;

    /// Serializes any test that mutates `ORBIT_AGENT_*` env vars or asserts on
    /// audit rows whose `role` depends on env-var precedence. Without this
    /// guard, cargo's parallel test harness can race two env writers and
    /// produce non-reproducible failures.
    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn clear_identity_env() {
        // SAFETY: tests serialize through `env_guard()` before calling this.
        unsafe {
            std::env::remove_var("ORBIT_AGENT_NAME");
            std::env::remove_var("ORBIT_AGENT_MODEL");
        }
    }

    fn set_identity_env(agent: &str, model: &str) {
        // SAFETY: tests serialize through `env_guard()` before calling this.
        unsafe {
            std::env::set_var("ORBIT_AGENT_NAME", agent);
            std::env::set_var("ORBIT_AGENT_MODEL", model);
        }
    }

    fn fresh_runtime() -> OrbitRuntime {
        // Reset the dedup signal so cross-test thread-local leakage cannot
        // mask real bugs in the per-call set/clear cycle.
        let _ = take_tool_audit_recorded();
        clear_identity_env();
        OrbitRuntime::in_memory().expect("build in-memory runtime")
    }

    #[test]
    fn dispatch_records_success_audit_with_mcp_subcommand_and_clamped_duration() {
        let _g = env_guard();
        let runtime = fresh_runtime();

        let outcome = runtime
            .execute_tool_command_dispatch(
                "orbit.search",
                json!({ "query": "anything", "model": "gpt-5.5" }),
                None,
                None,
                ToolEntryPoint::Mcp,
            )
            .expect("dispatch ok");
        assert!(outcome.audit_recorded);

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "exactly one audit row");
        let row = &events[0];
        assert_eq!(row.command, "tool");
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(row.tool_name.as_deref(), Some("orbit.search"));
        assert_eq!(row.target_type.as_deref(), Some("tool"));
        assert_eq!(row.target_id.as_deref(), Some("orbit.search"));
        assert_eq!(row.role, "gpt-5.5");
        assert_eq!(row.status, AuditEventStatus::Success);
        assert_eq!(row.exit_code, 0);
        assert!(
            row.duration_ms >= 1,
            "duration_ms clamped to >= 1 (got {})",
            row.duration_ms
        );
    }

    #[test]
    fn dispatch_records_failure_audit_when_tool_handler_errors() {
        let _g = env_guard();
        let runtime = fresh_runtime();

        // Missing required input fields makes the task tool error out at
        // dispatch time. That gives us a deterministic dispatch-failure path
        // that runs through the runtime audit-write seam.
        let result = runtime.execute_tool_command_dispatch(
            "orbit.task.show",
            json!({}),
            None,
            None,
            ToolEntryPoint::Mcp,
        );
        assert!(result.is_err(), "dispatch errors with missing input");

        let events = runtime
            .list_audit_events(None, Some("orbit.task.show".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        let row = &events[0];
        assert_eq!(row.status, AuditEventStatus::Failure);
        assert_eq!(row.exit_code, 1);
        assert!(row.error_message.is_some());
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
    }

    #[test]
    fn dispatch_records_failure_audit_when_identity_setup_rejects_pair() {
        let _g = env_guard();
        let runtime = fresh_runtime();

        // Inconsistent agent/model: `claude` family does not produce
        // `gpt-5.5`. `resolve_agent_identity` rejects this via
        // `normalize_agent_family_for_model`. The audit-write path must
        // still capture the failure — this is the gap that bypassed audit
        // before the closure-wrapping fix.
        let result = runtime.execute_tool_command_dispatch(
            "orbit.search",
            json!({ "query": "anything" }),
            Some("claude".to_string()),
            Some("gpt-5.5".to_string()),
            ToolEntryPoint::Mcp,
        );
        assert!(result.is_err(), "identity rejection propagates");

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(
            events.len(),
            1,
            "setup failure produced exactly one audit row"
        );
        let row = &events[0];
        assert_eq!(row.status, AuditEventStatus::Failure);
        assert_eq!(row.exit_code, 1);
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
        assert!(row.error_message.is_some(), "error message captured");
    }

    #[test]
    fn cli_entry_point_records_run_subcommand() {
        let _g = env_guard();
        let runtime = fresh_runtime();

        runtime
            .execute_tool_command(
                "orbit.search",
                json!({ "query": "anything", "model": "gpt-5.5" }),
                None,
                None,
            )
            .expect("dispatch ok");

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subcommand.as_deref(), Some("run"));
    }

    #[test]
    fn concurrent_tool_dispatch_writes_distinct_execution_ids() {
        let _g = env_guard();
        let runtime = Arc::new(fresh_runtime());
        let workers = 8;
        let barrier = Arc::new(Barrier::new(workers));

        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    runtime
                        .execute_tool_command_dispatch(
                            "orbit.search",
                            json!({ "query": "anything", "model": "gpt-5.5" }),
                            None,
                            None,
                            ToolEntryPoint::Cli,
                        )
                        .expect("dispatch ok");
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("worker joined");
        }

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, workers)
            .expect("list audit events");
        let execution_ids: BTreeSet<_> = events.iter().map(|event| &event.execution_id).collect();

        assert_eq!(events.len(), workers);
        assert_eq!(execution_ids.len(), workers);
    }

    #[test]
    fn dedup_signal_is_set_after_dispatch_and_cleared_on_take() {
        let _g = env_guard();
        let runtime = fresh_runtime();
        let _ = take_tool_audit_recorded();
        assert!(!take_tool_audit_recorded(), "starts clear");

        runtime
            .execute_tool_command_dispatch(
                "orbit.search",
                json!({ "query": "anything" }),
                None,
                None,
                ToolEntryPoint::Cli,
            )
            .expect("dispatch ok");

        assert!(
            take_tool_audit_recorded(),
            "runtime sets flag after audit write"
        );
        assert!(
            !take_tool_audit_recorded(),
            "take is one-shot and resets the flag"
        );
    }

    #[test]
    fn audit_role_label_prefers_input_json_over_flags_and_env() {
        let _g = env_guard();
        // Set env vars to a value we never expect to see, so a leak surfaces
        // as a test failure with a recognizable string.
        set_identity_env("env-leak", "env-leak-model");
        let role = audit_role_label(
            &json!({ "agent": "claude", "model": "opus-4.6" }),
            Some("codex"),
            Some("gpt-5.5"),
        );
        clear_identity_env();
        assert_eq!(role, "opus-4.6");
    }

    #[test]
    fn audit_role_label_prefers_flags_over_env_when_input_absent() {
        let _g = env_guard();
        set_identity_env("env-leak", "env-leak-model");
        let role = audit_role_label(&json!({ "query": "x" }), Some("codex"), Some("gpt-5.5"));
        clear_identity_env();
        assert_eq!(role, "gpt-5.5");
    }

    #[test]
    fn audit_role_label_falls_back_to_env_when_input_and_flags_absent() {
        let _g = env_guard();
        set_identity_env("claude", "opus-4.6");
        let role = audit_role_label(&json!({ "query": "x" }), None, None);
        clear_identity_env();
        assert_eq!(role, "claude");
    }

    #[test]
    fn audit_role_label_overwrites_self_reported_model_with_env_family() {
        let _g = env_guard();
        set_identity_env("claude", "claude-opus-4-7");
        let role = audit_role_label(&json!({ "model": "opus-4.7" }), None, None);
        clear_identity_env();
        assert_eq!(role, "claude");
    }

    #[test]
    fn cli_tool_dispatch_env_identity_overwrites_task_update_self_reported_model() {
        let _g = env_guard();
        let runtime = fresh_runtime();
        let task = runtime
            .add_task(crate::command::task::TaskAddParams {
                title: "identity regression".to_string(),
                description: "exercise CLI tool identity overwrite".to_string(),
                acceptance_criteria: vec!["implemented_by is canonical".to_string()],
                plan: "Do the work.".to_string(),
                status: Some(orbit_common::types::TaskStatus::InProgress),
                ..Default::default()
            })
            .expect("seed in-progress task");
        set_identity_env("grok", "grok-build");

        runtime
            .execute_tool_command_dispatch(
                "orbit.task.update",
                json!({
                    "id": task.id.clone(),
                    "status": "review",
                    "execution_summary": "Done.",
                    "model": "claude-opus-4-7"
                }),
                None,
                None,
                ToolEntryPoint::Cli,
            )
            .expect("task update succeeds");
        clear_identity_env();

        let updated = runtime.get_task(&task.id).expect("read updated task");
        assert_eq!(updated.implemented_by.as_deref(), Some("grok"));
    }

    #[test]
    fn audit_role_label_defaults_to_agent_when_no_identity_available() {
        let _g = env_guard();
        clear_identity_env();
        let role = audit_role_label(&json!({}), None, None);
        assert_eq!(role, "agent");
    }

    fn clear_audit_context_env() {
        // SAFETY: tests serialize through `env_guard()` before calling this.
        unsafe {
            std::env::remove_var("ORBIT_TASK_ID");
            std::env::remove_var("ORBIT_RUN_ID");
            std::env::remove_var(ORBIT_MANAGED_RUN_CONTEXT_ENV);
            std::env::remove_var("ORBIT_ACTIVITY_ID");
            std::env::remove_var("ORBIT_STEP_INDEX");
        }
    }

    fn set_audit_context_env(task: &str, run: &str, activity: &str, step: &str) {
        // SAFETY: tests serialize through `env_guard()` before calling this.
        unsafe {
            std::env::set_var("ORBIT_TASK_ID", task);
            std::env::set_var("ORBIT_RUN_ID", run);
            std::env::set_var("ORBIT_ACTIVITY_ID", activity);
            std::env::set_var("ORBIT_STEP_INDEX", step);
        }
    }

    #[test]
    fn audit_context_input_wins_over_env() {
        let _g = env_guard();
        set_audit_context_env("env-task", "env-run", "env-activity", "9");
        let ctx = resolve_audit_context(&json!({
            "task_id": "T-input",
            "job_run_id": "jrun-input",
            "activity_id": "act-input",
            "step_index": 3,
        }));
        clear_audit_context_env();

        assert_eq!(ctx.task_id.as_deref(), Some("T-input"));
        assert_eq!(ctx.job_run_id.as_deref(), Some("jrun-input"));
        assert_eq!(ctx.activity_id.as_deref(), Some("act-input"));
        assert_eq!(ctx.step_index, Some(3));
    }

    #[test]
    fn audit_context_falls_back_to_env_when_input_absent() {
        let _g = env_guard();
        set_audit_context_env("T20260428-7", "jrun-from-env", "agent_implement", "2");
        let ctx = resolve_audit_context(&json!({}));
        clear_audit_context_env();

        assert_eq!(ctx.task_id.as_deref(), Some("T20260428-7"));
        assert_eq!(ctx.job_run_id.as_deref(), Some("jrun-from-env"));
        assert_eq!(ctx.activity_id.as_deref(), Some("agent_implement"));
        assert_eq!(ctx.step_index, Some(2));
    }

    #[test]
    fn audit_context_treats_run_id_alias_as_job_run_id_input() {
        let _g = env_guard();
        clear_audit_context_env();
        let ctx = resolve_audit_context(&json!({ "run_id": "jrun-aliased" }));
        assert_eq!(ctx.job_run_id.as_deref(), Some("jrun-aliased"));
    }

    #[test]
    fn reservation_owner_context_ignores_unmanaged_orbit_run_env() {
        let _g = env_guard();
        clear_audit_context_env();
        // SAFETY: tests serialize through `env_guard()` before mutating env.
        unsafe {
            std::env::set_var("ORBIT_RUN_ID", "jrun-env-owner");
        }

        assert_eq!(reservation_owner_from_env(), None);
        clear_audit_context_env();
    }

    #[test]
    fn reservation_owner_context_comes_from_managed_orbit_run_env() {
        let _g = env_guard();
        clear_audit_context_env();
        // SAFETY: tests serialize through `env_guard()` before mutating env.
        unsafe {
            std::env::set_var("ORBIT_RUN_ID", "jrun-env-owner");
            std::env::set_var(ORBIT_MANAGED_RUN_CONTEXT_ENV, "1");
        }
        let owner = reservation_owner_from_env().expect("owner from managed env");
        clear_audit_context_env();

        assert_eq!(owner.owner_run_id, "jrun-env-owner");
        assert!(
            owner
                .owner_metadata_json
                .as_deref()
                .is_some_and(|raw| { raw.contains("\"source\":\"orbit_cli\"") })
        );
    }

    #[test]
    fn audit_context_returns_none_when_neither_source_supplies_values() {
        let _g = env_guard();
        clear_audit_context_env();
        let ctx = resolve_audit_context(&json!({}));
        assert!(ctx.task_id.is_none());
        assert!(ctx.job_run_id.is_none());
        assert!(ctx.activity_id.is_none());
        assert!(ctx.step_index.is_none());
    }

    #[test]
    fn dispatch_records_correlation_fields_from_env() {
        let _g = env_guard();
        let runtime = fresh_runtime();
        set_audit_context_env("T20260428-7", "jrun-corr", "agent_implement", "5");

        let outcome = runtime
            .execute_tool_command_dispatch(
                "orbit.search",
                json!({ "query": "anything", "model": "gpt-5.5" }),
                None,
                None,
                ToolEntryPoint::Cli,
            )
            .expect("dispatch ok");
        clear_audit_context_env();
        assert!(outcome.audit_recorded);

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        let row = events
            .iter()
            .find(|e| e.execution_id.starts_with("exec-"))
            .expect("at least one row");
        assert_eq!(row.task_id.as_deref(), Some("T20260428-7"));
        assert_eq!(row.job_run_id.as_deref(), Some("jrun-corr"));
        assert_eq!(row.activity_id.as_deref(), Some("agent_implement"));
        assert_eq!(row.step_index, Some(5));
    }
}

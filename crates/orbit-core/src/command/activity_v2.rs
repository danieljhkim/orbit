//! Direct v2 activity execution helper.
//!
//! Reads a YAML file from disk, parses it through the two-pass loader at
//! `orbit_common::types::activity_job::load_activity_asset`, and invokes the dispatcher with
//! `OrbitRuntime` as the `V2RuntimeHost` (impl lives in
//! `crate::runtime::v2_host`).
//!
//! Loop + envelope audit sink construction is delegated to
//! `V2AuditWriter::with_disk_sinks` — this file never names orbit-agent types.

use std::path::{Path, PathBuf};

use orbit_common::types::activity_job::{
    Backend, V2AuditEventKind, load_activity_asset, resolve_activity_backends,
    validate_activity_tool_allowlist_against_registered_tools,
};
use orbit_common::types::{OrbitError, OrbitEvent};
use orbit_engine::{V2AuditWriter, V2DispatchInput, dispatch_v2_activity};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::SYSTEM_AUDIT_IDENTITY;

#[derive(Debug)]
pub struct V2ActivityRunResult {
    pub activity_name: String,
    pub activity_type: &'static str,
    pub success: bool,
    pub output: Value,
    pub message: Option<String>,
    pub audit_jsonl: Option<PathBuf>,
    pub events_emitted: usize,
    /// Resolved execution backend applied to the asset at load time. `None`
    /// when the activity isn't `agent_loop` (deterministic/shell ignore
    /// `backend:`).
    pub resolved_backend: Option<Backend>,
}

impl OrbitRuntime {
    /// Execute a v2 activity from a YAML path. Returns a structural result
    /// plus the path to the persisted §7 envelope JSONL.
    ///
    /// `backend_flag` is the `--backend` invocation-level override; when
    /// `None`, the resolver falls through to env → config → default.
    pub fn run_activity_v2_from_yaml(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
    ) -> Result<V2ActivityRunResult, OrbitError> {
        let yaml = std::fs::read_to_string(yaml_path).map_err(|err| {
            OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display()))
        })?;
        let mut asset = load_activity_asset(&yaml).map_err(|err| {
            OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display()))
        })?;
        let registered_tools: Vec<String> = self
            .tool_registry()
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect();
        validate_activity_tool_allowlist_against_registered_tools(
            &asset.spec,
            registered_tools.iter().map(String::as_str),
        )
        .map_err(|err| {
            OrbitError::InvalidInput(format!(
                "activity `{}` tool allowlist invalid: {err}",
                asset.name
            ))
        })?;

        // §3.1 resolution: replace `Auto` with a concrete backend per
        // precedence (flag → env → config → cli).
        let resolution = self.resolve_v2_backend(backend_flag);
        resolve_activity_backends(&mut asset.spec, resolution.backend);
        let resolved_backend = match &asset.spec.spec {
            orbit_common::types::activity_job::ActivityV2Spec::AgentLoop(spec) => {
                Some(spec.backend)
            }
            orbit_common::types::activity_job::ActivityV2Spec::Groundhog(_) => Some(Backend::Http),
            _ => None,
        };

        let run_id = format!(
            "activity-{}-{}",
            asset.name,
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f")
        );

        let audit_root = self.paths().audit_dir.clone();
        let workspace_path = self.paths().repo_root.clone();
        let writer = V2AuditWriter::with_disk_sinks(
            &audit_root,
            &run_id,
            SYSTEM_AUDIT_IDENTITY,
            Some(workspace_path.as_path()),
        )
        .map_err(|err| OrbitError::Execution(format!("audit sinks: {err}")))?;
        let audit_jsonl = writer.envelope_log_path();

        // Record the standard orbit-core activity-run lifecycle events so v2
        // runs appear in the same audit stream v1 runs use.
        self.record_event(OrbitEvent::ActivityRunStarted {
            id: asset.name.clone(),
        })?;
        let _ = writer.emit(V2AuditEventKind::RunStarted {
            job_name: format!("cli:{}", asset.name),
            retry_source_run_id: None,
        });

        let activity_type = match &asset.spec.spec {
            orbit_common::types::activity_job::ActivityV2Spec::AgentLoop(_) => "agent_loop",
            orbit_common::types::activity_job::ActivityV2Spec::Groundhog(_) => "groundhog",
            orbit_common::types::activity_job::ActivityV2Spec::Deterministic(_) => "deterministic",
            orbit_common::types::activity_job::ActivityV2Spec::Shell(_) => "shell",
        };

        let dispatch = dispatch_v2_activity(V2DispatchInput {
            activity_name: &asset.name,
            spec: &asset.spec.spec,
            fs_profile: asset.spec.fs_profile.as_deref(),
            input,
            audit: writer.clone(),
            run_id: &run_id,
            host: Some(self),
        });

        let (outcome_str, error_message) = match &dispatch {
            Ok(o) if o.success => ("success", None),
            Ok(o) => ("failed", o.message.clone()),
            Err(err) => ("error", Some(format!("v2 dispatch: {err}"))),
        };
        let _ = writer.emit(V2AuditEventKind::RunFinished {
            outcome: outcome_str.to_string(),
            error_message,
        });
        self.record_event(OrbitEvent::ActivityRunCompleted {
            id: asset.name.clone(),
            state: outcome_str.to_string(),
        })?;

        let events_count = writer
            .events_snapshot()
            .map(|s| s.len())
            .unwrap_or_default();

        match dispatch {
            Ok(o) => Ok(V2ActivityRunResult {
                activity_name: asset.name,
                activity_type,
                success: o.success,
                output: o.output,
                message: o.message,
                audit_jsonl,
                events_emitted: events_count,
                resolved_backend,
            }),
            Err(err) => Err(OrbitError::Execution(format!("v2 dispatch: {err}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, repo_root)
    }

    fn write_activity(path: &Path, name: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: deterministic
  description: Test deterministic sleep.
  action: sleep
  config: {{}}
"#
        );
        std::fs::write(path, yaml).expect("write activity yaml");
    }

    #[cfg(unix)]
    fn write_failing_shell_activity(path: &Path, name: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: shell
  description: Test failing shell.
  program: /bin/sh
  args: ["-c", "exit 7"]
  allowed_programs: ["/bin/sh"]
"#
        );
        std::fs::write(path, yaml).expect("write activity yaml");
    }

    fn write_agent_loop_activity(path: &Path, name: &str, tool: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: agent_loop
  description: Test agent loop.
  instruction: Test.
  tools:
    - {tool}
"#
        );
        std::fs::write(path, yaml).expect("write activity yaml");
    }

    #[test]
    fn direct_activity_run_uses_system_audit_identity() {
        let (_root, runtime, repo_root) = test_runtime();
        let yaml_path = repo_root.join("qa_activity_sleep.yaml");
        write_activity(&yaml_path, "qa_activity_sleep");

        let result = runtime
            .run_activity_v2_from_yaml(&yaml_path, json!({ "seconds": 0 }), None)
            .expect("direct activity run succeeds");

        let audit_jsonl = result.audit_jsonl.as_ref().expect("audit jsonl path");
        let first_line = std::fs::read_to_string(audit_jsonl)
            .expect("read audit jsonl")
            .lines()
            .next()
            .expect("audit jsonl has a first event")
            .to_string();
        let first_event: serde_json::Value =
            serde_json::from_str(&first_line).expect("parse first audit event");
        assert_eq!(
            first_event
                .get("agent_identity")
                .and_then(serde_json::Value::as_str),
            Some(SYSTEM_AUDIT_IDENTITY)
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_activity_run_finished_audit_carries_non_success_message() {
        let (_root, runtime, repo_root) = test_runtime();
        let yaml_path = repo_root.join("qa_activity_shell_fail.yaml");
        write_failing_shell_activity(&yaml_path, "qa_activity_shell_fail");

        let result = runtime
            .run_activity_v2_from_yaml(&yaml_path, json!({}), None)
            .expect("shell activity returns structural non-success");

        assert!(!result.success);
        assert_eq!(result.message.as_deref(), Some("exit 7 not in [0]"));
        let audit_jsonl = result.audit_jsonl.as_ref().expect("audit jsonl path");
        let run_finished = std::fs::read_to_string(audit_jsonl)
            .expect("read audit jsonl")
            .lines()
            .find(|line| line.contains(r#""body_kind":"run_finished""#))
            .expect("run_finished audit event")
            .to_string();
        let event: serde_json::Value =
            serde_json::from_str(&run_finished).expect("parse run_finished");
        assert_eq!(
            event.get("outcome").and_then(serde_json::Value::as_str),
            Some("failed")
        );
        assert_eq!(
            event
                .get("error_message")
                .and_then(serde_json::Value::as_str),
            Some("exit 7 not in [0]")
        );
    }

    #[test]
    fn direct_activity_run_rejects_unknown_tool_before_dispatch() {
        let (_root, runtime, repo_root) = test_runtime();
        let yaml_path = repo_root.join("unknown_tool_activity.yaml");
        write_agent_loop_activity(&yaml_path, "unknown_tool_activity", "orbit.task.nope");

        let err = runtime
            .run_activity_v2_from_yaml(&yaml_path, json!({}), None)
            .expect_err("unknown tool should fail before dispatch");
        let message = err.to_string();

        assert!(message.contains("unknown_tool_activity"), "{message}");
        assert!(message.contains("orbit.task.nope"), "{message}");
        assert!(message.contains("unknown tool name"), "{message}");
    }
}

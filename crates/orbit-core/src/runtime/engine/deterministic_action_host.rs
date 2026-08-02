use std::sync::Arc;

use orbit_common::types::{ActivityV2, AgentModelPair, JobRun, OrbitError, OrbitEvent, Role};
use orbit_engine::{DeterministicActionHost, V2AuditWriter, V2RuntimeHost};
use orbit_store::{InvocationQuery, InvocationRecord};
use orbit_tools::ToolContext;
use serde_json::Value;

use super::paths::current_repo_root;
use crate::OrbitRuntime;

impl DeterministicActionHost for OrbitRuntime {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        OrbitRuntime::record_event(self, event)
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        current_repo_root(self)
    }

    fn list_job_runs_for_gc(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs(crate::command::job::JobRunListParams::default())
    }

    fn data_root(&self) -> &std::path::Path {
        self.context.data_root()
    }

    fn managed_worktree_preparation_profile(
        &self,
        activity_name: &str,
    ) -> Result<Option<orbit_common::types::ResolvedFsProfile>, OrbitError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = activity_name;
            return Ok(None);
        }
        #[cfg(target_os = "linux")]
        {
            let activity = match self.v2_activity(activity_name) {
                Ok(activity) => activity,
                Err(OrbitError::InvalidInput(message))
                    if message == format!("v2 activity '{activity_name}' not found") =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            let profile_name = activity
                .fs_profile
                .as_deref()
                .unwrap_or(orbit_common::types::UNRESTRICTED_FS_PROFILE);
            let profile = self.policy_engine().def().effective_profile(profile_name)?;
            Ok(Some(profile))
        }
    }

    fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        OrbitRuntime::cancel_job_run(self, run_id).map(|_| ())
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.configured_agent_model_pair(agent_cli)
    }

    fn duel_candidate_families(&self) -> Vec<String> {
        self.duel_config().candidates.clone()
    }

    fn duel_orchestrator_model(&self, family: &str) -> Option<String> {
        let family = family.trim().to_ascii_lowercase();
        self.duel_config().models.get(&family).cloned()
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        let _ = agent_cli;
        model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        OrbitRuntime::invocation_records(self, query)
    }

    fn activity_implementer_identity(
        &self,
        input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        self.implementer_identity_for_activity_input(input)
    }

    fn resolved_crew_model(&self, run_id: &str) -> Result<Option<String>, OrbitError> {
        Ok(self
            .get_job_run_backend(run_id)?
            .and_then(|run| run.crew_model)
            .and_then(|model| {
                let model = model.trim();
                (!model.is_empty()).then(|| model.to_string())
            }))
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        OrbitRuntime::run_tool_with_context_and_role(self, name, input, role, tool_context)
    }

    fn v2_runtime_host(&self) -> Result<&dyn V2RuntimeHost, OrbitError> {
        Ok(self)
    }

    fn v2_activity(&self, name: &str) -> Result<ActivityV2, OrbitError> {
        self.v2_activity_catalog()
            .map_err(|error| {
                OrbitError::InvalidInput(format!("build v2 activity catalog: {error}"))
            })?
            .get(name)
            .cloned()
            .ok_or_else(|| OrbitError::InvalidInput(format!("v2 activity '{name}' not found")))
    }

    fn v2_audit_writer(&self, run_id: &str) -> Result<Arc<V2AuditWriter>, OrbitError> {
        V2AuditWriter::with_disk_sinks(
            &self.paths().audit_dir,
            self.sqlite_store()?,
            self.workspace_id()?,
            run_id,
            "system",
            Some(self.paths().repo_root.as_path()),
        )
        .map_err(|error| OrbitError::Execution(format!("v2 audit sinks: {error}")))
    }

    fn maybe_create_failure_task(
        &self,
        _job_id: &str,
        _run_id: &str,
        _error_code: &str,
        _error_message: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn scoring_enabled(&self) -> bool {
        self.context.scoring_enabled()
    }

    fn actor_model_identity(&self) -> Option<String> {
        matches!(self.actor().kind, crate::context::ActorKind::Agent)
            .then(|| self.actor_label().trim())
            .filter(|label| !label.is_empty())
            .map(ToOwned::to_owned)
    }

    fn pr_config(&self) -> orbit_engine::PrConfig {
        OrbitRuntime::pr_config(self).clone()
    }

    fn scoreboard_dir(&self) -> &std::path::Path {
        &self.context.paths().scoreboard_dir
    }
}

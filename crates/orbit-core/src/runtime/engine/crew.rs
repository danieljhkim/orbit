use orbit_common::types::{
    Crew, CrewRoleAssignment, OrbitError, Task, all_agent_families, infer_agent_family_from_model,
    resolve_crew,
};
use serde::Serialize;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::run_input::{non_empty, singular_task_id_from_input};

/// Runtime crew registry projection for dashboard/API consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfiguredCrewRegistryProjection {
    pub default_crew: Option<String>,
    pub crews: Vec<ConfiguredCrewProjection>,
}

/// Named crew and role-model strings from the active runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfiguredCrewProjection {
    pub name: String,
    pub planner_model: String,
    pub implementer_model: String,
    pub reviewer_model: String,
    pub is_default: bool,
}

impl ConfiguredCrewProjection {
    fn from_crew(crew: &Crew, is_default: bool) -> Self {
        Self {
            name: crew.name.clone(),
            planner_model: crew.planner.model.clone(),
            implementer_model: crew.implementer.model.clone(),
            reviewer_model: crew.reviewer.model.clone(),
            is_default,
        }
    }
}

/// Crew/role-model strings to surface on a task projection.
///
/// Decouples projection consumers from the full `Crew` type so this struct can
/// also be hydrated directly from persisted run-record fields, which carry only
/// the model strings (not provider/backend).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCrewProjection {
    pub name: String,
    pub planner_model: String,
    pub implementer_model: String,
    pub reviewer_model: String,
}

impl ResolvedCrewProjection {
    fn from_crew(crew: Crew) -> Self {
        Self {
            name: crew.name,
            planner_model: crew.planner.model,
            implementer_model: crew.implementer.model,
            reviewer_model: crew.reviewer.model,
        }
    }
}

impl OrbitRuntime {
    pub fn configured_crew_registry_projection(&self) -> ConfiguredCrewRegistryProjection {
        let default_crew = self.context.default_crew().map(ToString::to_string);
        let mut crews = self
            .context
            .crews()
            .values()
            .map(|crew| {
                ConfiguredCrewProjection::from_crew(
                    crew,
                    default_crew.as_deref() == Some(crew.name.as_str()),
                )
            })
            .collect::<Vec<_>>();
        crews.sort_by(|left, right| left.name.cmp(&right.name));
        ConfiguredCrewRegistryProjection {
            default_crew,
            crews,
        }
    }

    pub fn validate_crew_name(&self, crew: Option<&str>) -> Result<(), OrbitError> {
        let Some(crew) = crew.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(());
        };
        resolve_crew(crew, self.context.crews()).map(|_| ())
    }

    pub fn resolve_crew_for_task(
        &self,
        cli_override: Option<&str>,
        task_crew: Option<&str>,
    ) -> Result<Crew, OrbitError> {
        let selected = cli_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| task_crew.map(str::trim).filter(|value| !value.is_empty()))
            .or_else(|| self.context.default_crew());
        let Some(selected) = selected else {
            return Err(OrbitError::InvalidInput(
                "no crew selected; set [workflow].default_crew, task.crew, or pass crew"
                    .to_string(),
            ));
        };
        resolve_crew(selected, self.context.crews())
    }

    pub(crate) fn resolve_crew_for_run_input(&self, input: &Value) -> Result<Crew, OrbitError> {
        let cli_override = input
            .get("crew")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let task_crew = self
            .task_id_from_run_input(input)?
            .map(|task| task.crew)
            .unwrap_or_default();
        self.resolve_crew_for_task(cli_override, task_crew.as_deref())
    }

    /// Resolve a crew/role-model projection for `orbit.task.show` consumers.
    ///
    /// Selection truth comes first: when the task points at a run record that
    /// persisted the resolved crew, those four strings win — they reflect what
    /// was selected for routing, even if the workspace registry has been edited
    /// since. "Who actually ran?" projections read invocation records instead.
    ///
    /// Best-effort otherwise: if neither the task nor the workspace can name a
    /// crew, `Ok(None)` so readers (CLI, MCP) can omit the fields instead of
    /// failing the entire task readout. Genuine misconfigurations (stale crew
    /// name in `task.crew` or `default_crew`) still surface as `Err`.
    pub fn resolved_crew_projection(
        &self,
        task: &Task,
    ) -> Result<Option<ResolvedCrewProjection>, OrbitError> {
        if let Some(run_id) = task.job_run_id.as_deref()
            && let Some(run) = self.get_job_run_backend(run_id)?
            && let (
                Some(resolved_crew),
                Some(planner_model),
                Some(implementer_model),
                Some(reviewer_model),
            ) = (
                run.resolved_crew,
                run.planner_model,
                run.implementer_model,
                run.reviewer_model,
            )
        {
            return Ok(Some(ResolvedCrewProjection {
                name: resolved_crew,
                planner_model,
                implementer_model,
                reviewer_model,
            }));
        }

        let has_resolvable_name = task.crew.is_some() || self.context.default_crew().is_some();
        if !has_resolvable_name {
            return Ok(None);
        }

        self.resolve_crew_for_task(None, task.crew.as_deref())
            .map(ResolvedCrewProjection::from_crew)
            .map(Some)
    }

    pub(crate) fn record_run_crew_from_input(
        &self,
        run_id: &str,
        input: &Value,
    ) -> Result<Crew, OrbitError> {
        let crew = self.resolve_crew_for_run_input(input)?;
        tracing::info!(
            run_id,
            resolved_crew = %crew.name,
            planner_model = %crew.planner.model,
            implementer_model = %crew.implementer.model,
            reviewer_model = %crew.reviewer.model,
            "crew resolved for run",
        );
        self.stores().jobs().record_run_crew(run_id, &crew)?;
        Ok(crew)
    }

    pub(crate) fn implementer_identity_for_activity_input(
        &self,
        input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        let task = self.task_id_from_run_input(input)?;
        let input_run_id = input
            .get("run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let task_run_id = task.as_ref().and_then(|task| task.job_run_id.as_deref());
        let Some(run_id) = input_run_id.or(task_run_id) else {
            return Ok((None, None));
        };
        let Some(run) = self.get_job_run_backend(run_id)? else {
            return Ok((None, None));
        };

        if let Some(crew_name) = run
            .resolved_crew
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && let Ok(crew) = resolve_crew(crew_name, self.context.crews())
            && let Some(family) = family_from_assignment(&crew.implementer)
        {
            return Ok((Some(family.clone()), Some(family)));
        }

        if let Some(family) = run
            .implementer_model
            .as_deref()
            .and_then(infer_agent_family_from_model)
        {
            return Ok((Some(family.clone()), Some(family)));
        }

        Ok((None, None))
    }

    pub(crate) fn resolve_and_log_crew_for_task_start(
        &self,
        task_id: &str,
        crew_override: Option<&str>,
        task_crew: Option<&str>,
    ) -> Result<Crew, OrbitError> {
        let crew = self.resolve_crew_for_task(crew_override, task_crew)?;
        tracing::info!(
            task_id,
            resolved_crew = %crew.name,
            planner_model = %crew.planner.model,
            implementer_model = %crew.implementer.model,
            reviewer_model = %crew.reviewer.model,
            "crew resolved for task start",
        );
        Ok(crew)
    }

    fn task_id_from_run_input(&self, input: &Value) -> Result<Option<Task>, OrbitError> {
        if let Some(task_id) = singular_task_id_from_input(input)
            && task_id.starts_with("ORB-")
        {
            return self.get_task(task_id).map(Some);
        }

        for key in ["task_id", "taskId", "id"] {
            let Some(task_id) = input.get(key).and_then(Value::as_str) else {
                continue;
            };
            let Some(task_id) = non_empty(task_id) else {
                continue;
            };
            if !task_id.starts_with("ORB-") {
                continue;
            }
            return self.get_task(task_id).map(Some);
        }
        Ok(None)
    }
}

fn family_from_assignment(assignment: &CrewRoleAssignment) -> Option<String> {
    let provider = assignment.provider.trim().to_ascii_lowercase();
    if all_agent_families()
        .iter()
        .any(|family| *family == provider)
    {
        return Some(provider);
    }

    infer_agent_family_from_model(&assignment.model)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    use crate::OrbitRuntime;
    use crate::command::task::TaskAddParams;

    fn runtime_with_named_crews() -> (TempDir, OrbitRuntime) {
        let root = tempdir().expect("create temp root");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(
            workspace_root.join("config.toml"),
            r#"
[crews.opus-codex]
planner = { model = "default-planner", provider = "codex", backend = "cli" }
implementer = { model = "default-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "default-reviewer", provider = "codex", backend = "cli" }

[crews.silver]
planner = { model = "silver-planner", provider = "codex", backend = "cli" }
implementer = { model = "silver-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "silver-reviewer", provider = "codex", backend = "cli" }

[crews.bronze]
planner = { model = "bronze-planner", provider = "codex", backend = "cli" }
implementer = { model = "bronze-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "bronze-reviewer", provider = "codex", backend = "cli" }

[workflow]
default_crew = "opus-codex"
"#,
        )
        .expect("write test config");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
        (root, runtime)
    }

    fn add_task_with_crew(runtime: &OrbitRuntime, crew: &str) -> String {
        runtime
            .add_task(TaskAddParams {
                title: format!("{crew} task"),
                description: "Task fixture for crew resolution.".to_string(),
                crew: Some(crew.to_string()),
                ..Default::default()
            })
            .expect("add task")
            .id
    }

    #[test]
    fn run_input_task_ids_singleton_resolves_task_crew() {
        let (_root, runtime) = runtime_with_named_crews();
        let task_id = add_task_with_crew(&runtime, "silver");

        let crew = runtime
            .resolve_crew_for_run_input(&json!({ "task_ids": [task_id] }))
            .expect("resolve crew");

        assert_eq!(crew.name, "silver");
        assert_eq!(crew.planner.model, "silver-planner");
        assert_eq!(crew.implementer.model, "silver-implementer");
        assert_eq!(crew.reviewer.model, "silver-reviewer");
    }

    #[test]
    fn record_run_crew_persists_singleton_task_ids_task_crew_models() {
        let (_root, runtime) = runtime_with_named_crews();
        let task_id = add_task_with_crew(&runtime, "silver");
        let input = json!({ "task_ids": [task_id] });
        let run = runtime
            .stores()
            .jobs()
            .insert_run("agent_implement", 1, Utc::now(), Some(input.clone()), None)
            .expect("insert run");

        let crew = runtime
            .record_run_crew_from_input(&run.run_id, &input)
            .expect("record crew");
        let stored = runtime.show_job_run(&run.run_id).expect("show stored run");

        assert_eq!(crew.name, "silver");
        assert_eq!(stored.resolved_crew.as_deref(), Some("silver"));
        assert_eq!(stored.planner_model.as_deref(), Some("silver-planner"));
        assert_eq!(
            stored.implementer_model.as_deref(),
            Some("silver-implementer")
        );
        assert_eq!(stored.reviewer_model.as_deref(), Some("silver-reviewer"));
    }

    #[test]
    fn explicit_crew_override_wins_over_singleton_task_ids_task_crew() {
        let (_root, runtime) = runtime_with_named_crews();
        let task_id = add_task_with_crew(&runtime, "silver");

        let crew = runtime
            .resolve_crew_for_run_input(&json!({
                "crew": "bronze",
                "task_ids": [task_id]
            }))
            .expect("resolve crew");

        assert_eq!(crew.name, "bronze");
        assert_eq!(crew.implementer.model, "bronze-implementer");
    }

    #[test]
    fn multi_task_ids_without_override_falls_back_to_default_crew() {
        let (_root, runtime) = runtime_with_named_crews();
        let silver_task_id = add_task_with_crew(&runtime, "silver");
        let bronze_task_id = add_task_with_crew(&runtime, "bronze");

        let crew = runtime
            .resolve_crew_for_run_input(&json!({
                "task_ids": [silver_task_id, bronze_task_id]
            }))
            .expect("resolve crew");

        assert_eq!(crew.name, "opus-codex");
        assert_eq!(crew.implementer.model, "default-implementer");
    }
}

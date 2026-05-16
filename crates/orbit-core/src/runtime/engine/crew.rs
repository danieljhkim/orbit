use orbit_common::types::{Crew, OrbitError, Task, resolve_crew};
use serde::Serialize;
use serde_json::Value;

use crate::OrbitRuntime;

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
    /// Audit-trail truth comes first: when the task points at a run record that
    /// persisted the resolved crew, those four strings win — they reflect what
    /// actually ran, even if the workspace registry has been edited since.
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
        for key in ["task_id", "taskId", "id"] {
            let Some(task_id) = input.get(key).and_then(Value::as_str) else {
                continue;
            };
            let task_id = task_id.trim();
            if !task_id.starts_with("ORB-") {
                continue;
            }
            return self.get_task(task_id).map(Some);
        }
        Ok(None)
    }
}

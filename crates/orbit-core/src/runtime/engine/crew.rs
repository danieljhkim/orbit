use orbit_common::types::activity_job::ProviderSource;
use orbit_common::types::{
    Crew, CrewAssignment, OrbitError, Task, all_agent_families, infer_agent_family_from_model,
    resolve_crew,
};
use serde::Serialize;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::run_input::{non_empty, singular_task_id_from_input};

/// Select the crew *name* to dispatch by the Constellation provider-resolution
/// precedence (ORB-10091, contract §3): explicit > task_config >
/// workspace_default > environment_default > system_default. Empty /
/// whitespace-only values are skipped (not a selection). Returns the chosen
/// name and the tier it came from; `None` only when no tier supplies a value.
/// Pure and table-tested so the precedence cannot drift from the shared
/// contract or from the `Provider::resolve` surface it mirrors.
pub(crate) fn select_crew_name<'a>(
    explicit: Option<&'a str>,
    task_config: Option<&'a str>,
    workspace_default: Option<&'a str>,
    environment_default: Option<&'a str>,
    system_default: Option<&'a str>,
) -> Option<(&'a str, ProviderSource)> {
    [
        (explicit, ProviderSource::Explicit),
        (task_config, ProviderSource::TaskConfig),
        (workspace_default, ProviderSource::WorkspaceDefault),
        (environment_default, ProviderSource::EnvironmentDefault),
        (system_default, ProviderSource::SystemDefault),
    ]
    .into_iter()
    .find_map(|(value, source)| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| (value, source))
    })
}

/// Runtime crew registry projection for dashboard/API consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfiguredCrewRegistryProjection {
    pub default_crew: Option<String>,
    pub crews: Vec<ConfiguredCrewProjection>,
}

/// Named crew and model string from the active runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfiguredCrewProjection {
    pub name: String,
    pub model: String,
    pub provider: String,
    pub backend: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub is_default: bool,
}

impl ConfiguredCrewProjection {
    fn from_crew(crew: &Crew, is_default: bool) -> Self {
        Self {
            name: crew.name.clone(),
            model: crew.assignment.model.clone(),
            provider: crew.assignment.provider.clone(),
            backend: crew.assignment.backend.clone(),
            description: crew.description.clone(),
            tags: crew.tags.clone(),
            is_default,
        }
    }
}

/// Crew/model strings to surface on a task projection.
///
/// Decouples projection consumers from the full `Crew` type so this struct can
/// also be hydrated directly from persisted run-record fields, which carry only
/// the model string (not provider/backend).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCrewProjection {
    pub name: String,
    pub model: String,
}

impl ResolvedCrewProjection {
    fn from_crew(crew: Crew) -> Self {
        Self {
            name: crew.name,
            model: crew.assignment.model,
        }
    }
}

impl OrbitRuntime {
    pub fn configured_crew_registry_projection(&self) -> ConfiguredCrewRegistryProjection {
        let default_crew = self
            .context
            .settings()
            .default_crew()
            .map(ToString::to_string);
        let mut crews = self
            .context
            .settings()
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
        self.canonical_crew_name(crew).map(|_| ())
    }

    /// Resolve a user-supplied crew name to the exact alias stored in the
    /// active named-crew registry. Blank optional values remain unset.
    pub(crate) fn canonical_crew_name(
        &self,
        crew: Option<&str>,
    ) -> Result<Option<String>, OrbitError> {
        let Some(crew) = crew.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        resolve_crew(crew, self.context.settings().crews()).map(|crew| Some(crew.name))
    }

    pub fn resolve_crew_for_task(
        &self,
        cli_override: Option<&str>,
        task_crew: Option<&str>,
    ) -> Result<Crew, OrbitError> {
        // Runtime precedence is explicit > task_config > the default projected
        // by RuntimeConfig. That projection has already resolved workspace >
        // environment > system-default precedence, so lower tiers are not
        // re-read here and cannot drift during a run.
        let selection = select_crew_name(
            cli_override,
            task_crew,
            self.context.settings().default_crew(),
            None,
            None,
        );

        let Some((selected, _source)) = selection else {
            return Err(OrbitError::InvalidInput(
                "no crew selected; set [workflow].default_crew, task.crew, or pass crew"
                    .to_string(),
            ));
        };
        resolve_crew(selected, self.context.settings().crews())
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
    /// persisted the resolved crew, those two strings win — they reflect what
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
            && let (Some(resolved_crew), Some(model)) = (run.resolved_crew, run.crew_model)
        {
            return Ok(Some(ResolvedCrewProjection {
                name: resolved_crew,
                model,
            }));
        }

        let has_resolvable_name =
            task.crew.is_some() || self.context.settings().default_crew().is_some();
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
            crew_model = %crew.assignment.model,
            "crew resolved for run",
        );
        self.stores().jobs().record_job_run_crew(run_id, &crew)?;
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
            && let Ok(crew) = resolve_crew(crew_name, self.context.settings().crews())
            && let Some(family) = family_from_assignment(&crew.assignment)
        {
            return Ok((Some(family.clone()), Some(family)));
        }

        if let Some(family) = run
            .crew_model
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
            crew_model = %crew.assignment.model,
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

fn family_from_assignment(assignment: &CrewAssignment) -> Option<String> {
    let provider = assignment.provider.trim().to_ascii_lowercase();
    if all_agent_families()
        .iter()
        .any(|family| *family == provider)
    {
        return Some(provider);
    }

    infer_agent_family_from_model(&assignment.model)
}

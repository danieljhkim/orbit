//! Auto-task CRUD [ORB-10149]: the shared domain surface behind both the CLI
//! (`orbit auto-task …`) and the MCP tools (`orbit.auto_task.*`). Definitions
//! are git-versioned YAML under `<orbit_dir>/auto_tasks/<name>.yaml`; these
//! methods are the single choke point that reads/writes them, so both entry
//! points stay consistent. Disabling is a `toggle`, never a delete.
//!
//! `generate` (CLI-only by design — see `docs/design/mcp-bridge/2_design.md`)
//! rides here too: it mints a task from a definition on demand by reusing the
//! scheduler's mint path, so there is exactly one template→task mapping.

use orbit_common::types::{
    AUTO_TASK_SCHEMA_VERSION, AutoTaskDefinition, AutoTaskSchedule, AutoTaskTemplate, DedupePolicy,
    OrbitError, Task,
};
use orbit_common::utility::fs::write_text_with_parent;

use crate::OrbitRuntime;

use super::loader::{collect_auto_tasks, definition_path};
use super::schedule::validate_schedule;
use super::scheduler::mint_task;

/// Parameters for creating a definition.
#[derive(Debug, Clone)]
pub struct AutoTaskAddParams {
    pub name: String,
    pub description: String,
    pub schedule: AutoTaskSchedule,
    pub template: AutoTaskTemplate,
    pub dedupe: DedupePolicy,
}

/// Present-field patch for updating a definition. Absent fields are unchanged;
/// `enabled` is patched through [`OrbitRuntime::auto_task_toggle`].
#[derive(Debug, Clone, Default)]
pub struct AutoTaskUpdateParams {
    pub description: Option<String>,
    pub schedule: Option<AutoTaskSchedule>,
    pub dedupe: Option<DedupePolicy>,
    pub template: Option<AutoTaskTemplate>,
}

impl OrbitRuntime {
    /// Create a new auto-task definition. Fails if a definition with the same
    /// name already exists (update or toggle it instead).
    pub fn auto_task_add(
        &self,
        params: AutoTaskAddParams,
    ) -> Result<AutoTaskDefinition, OrbitError> {
        let now = chrono::Utc::now().to_rfc3339();
        let actor = self.actor_label().to_string();
        let definition = AutoTaskDefinition {
            schema_version: AUTO_TASK_SCHEMA_VERSION,
            name: params.name,
            description: params.description,
            enabled: true,
            schedule: params.schedule,
            template: params.template,
            dedupe: params.dedupe,
            created_by: Some(actor.clone()),
            created_at: now.clone(),
            updated_by: Some(actor),
            updated_at: now,
        };
        self.validate_auto_task(&definition)?;

        let path = definition_path(&self.paths().orbit_dir, &definition.name);
        if path.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "auto-task '{}' already exists; update or toggle it instead",
                definition.name
            )));
        }
        self.write_auto_task(&definition)?;
        Ok(definition)
    }

    /// List every definition in this workspace (stable filename order).
    /// Fail-closed load errors are surfaced as an error only when nothing
    /// loaded; otherwise malformed files are simply skipped by the loader.
    pub fn auto_task_list(&self) -> Result<Vec<AutoTaskDefinition>, OrbitError> {
        let collection = collect_auto_tasks(&self.paths().orbit_dir);
        Ok(collection
            .definitions
            .into_iter()
            .map(|loaded| loaded.definition)
            .collect())
    }

    /// Show one definition by name, or `None` if it does not exist.
    pub fn auto_task_show(&self, name: &str) -> Result<Option<AutoTaskDefinition>, OrbitError> {
        let path = definition_path(&self.paths().orbit_dir, name);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
        Ok(Some(orbit_common::types::parse_auto_task_yaml(&raw)?))
    }

    /// Apply a present-field patch to a definition.
    pub fn auto_task_update(
        &self,
        name: &str,
        params: AutoTaskUpdateParams,
    ) -> Result<AutoTaskDefinition, OrbitError> {
        let mut definition = self.require_auto_task(name)?;
        if let Some(description) = params.description {
            definition.description = description;
        }
        if let Some(schedule) = params.schedule {
            definition.schedule = schedule;
        }
        if let Some(dedupe) = params.dedupe {
            definition.dedupe = dedupe;
        }
        if let Some(template) = params.template {
            definition.template = template;
        }
        self.stamp_and_write(definition)
    }

    /// Enable or disable a definition (the kill-switch). Disabling is how an
    /// auto-task is retired — the definition and its history are preserved.
    pub fn auto_task_toggle(
        &self,
        name: &str,
        enabled: bool,
    ) -> Result<AutoTaskDefinition, OrbitError> {
        let mut definition = self.require_auto_task(name)?;
        definition.enabled = enabled;
        self.stamp_and_write(definition)
    }

    /// Mint one task from a definition on demand — the manual counterpart to a
    /// scheduler fire, so a new or edited definition can be exercised without
    /// waiting for its cron slot [ORB-10439].
    ///
    /// The mint is **unconditional**: schedule due-math, `dedupe`, and
    /// `enabled` are all ignored, and the host-local cursor at
    /// `<orbit_dir>/state/auto-tasks.json` is neither read nor written — an
    /// operator naming a definition explicitly means it, and a manual mint must
    /// not perturb scheduler state. Because it reuses the scheduler's
    /// [`mint_task`], the result is field-for-field identical to a fired
    /// instance, provenance tag and `system_created` marker included; that also
    /// means an open generated instance is visible to `skip_if_open` dedupe on
    /// the next pass, exactly as a fired one would be.
    ///
    /// An unknown name is an `InvalidInput` error naming the definition.
    pub fn auto_task_generate(&self, name: &str) -> Result<Task, OrbitError> {
        let definition = self.require_auto_task(name)?;
        mint_task(self, &definition)
    }

    fn require_auto_task(&self, name: &str) -> Result<AutoTaskDefinition, OrbitError> {
        self.auto_task_show(name)?
            .ok_or_else(|| OrbitError::InvalidInput(format!("no such auto-task '{name}'")))
    }

    fn stamp_and_write(
        &self,
        mut definition: AutoTaskDefinition,
    ) -> Result<AutoTaskDefinition, OrbitError> {
        definition.updated_by = Some(self.actor_label().to_string());
        definition.updated_at = chrono::Utc::now().to_rfc3339();
        self.validate_auto_task(&definition)?;
        self.write_auto_task(&definition)?;
        Ok(definition)
    }

    fn validate_auto_task(&self, definition: &AutoTaskDefinition) -> Result<(), OrbitError> {
        definition.validate()?;
        // Load-time cron validation happens in the scheduler, but validating
        // here too means CRUD never persists a schedule the scheduler would
        // reject at fire time.
        validate_schedule(&definition.schedule)?;
        if let Some(crew) = definition.template.crew.as_deref() {
            self.validate_crew_name(Some(crew))?;
        }
        Ok(())
    }

    fn write_auto_task(&self, definition: &AutoTaskDefinition) -> Result<(), OrbitError> {
        let path = definition_path(&self.paths().orbit_dir, &definition.name);
        let yaml = serde_yaml::to_string(definition).map_err(|error| {
            OrbitError::Io(format!("encode auto-task '{}': {error}", definition.name))
        })?;
        write_text_with_parent(&path, &yaml)
            .map_err(|error| OrbitError::Io(format!("write {}: {error}", path.display())))
    }
}

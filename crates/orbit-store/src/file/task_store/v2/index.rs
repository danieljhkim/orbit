use super::*;

impl TaskV2Store {
    pub(super) fn indexed_tasks(
        &self,
        filter: TaskIndexFilter,
    ) -> Result<Option<Vec<Task>>, OrbitError> {
        if !self.index_is_usable()? {
            return Ok(None);
        }
        let ids = self
            .registry
            .indexed_task_ids_filtered(&self.workspace_id, &filter)?;
        self.tasks_from_ids(ids).map(Some)
    }

    fn index_is_usable(&self) -> Result<bool, OrbitError> {
        let registered = self.registry.tasks_for_workspace(&self.workspace_id)?;
        let indexed = self
            .registry
            .indexed_task_versions_for_workspace(&self.workspace_id)?;
        if registered.len() != indexed.len() {
            return self.rebuild_index_best_effort("index count mismatch");
        }

        for binding in registered {
            let Some(indexed_updated_at) = indexed.get(&binding.task_id) else {
                return self.rebuild_index_best_effort("missing index row");
            };
            let bundle = self.read_existing_bundle(&binding.task_id)?;
            if bundle.envelope.updated_at.to_rfc3339() != *indexed_updated_at {
                return self.rebuild_index_best_effort("stale index row");
            }
        }
        Ok(true)
    }

    fn rebuild_index_best_effort(&self, reason: &str) -> Result<bool, OrbitError> {
        let bundles = self.bundle_store.list_bundles()?;
        let envelopes = bundles
            .into_iter()
            .map(|bundle| bundle.envelope)
            .collect::<Vec<_>>();
        match self
            .registry
            .replace_workspace_task_indexes(&self.workspace_id, &envelopes)
        {
            Ok(()) => Ok(true),
            Err(err) => {
                orbit_common::tracing::warn!(
                    target: "orbit.store.task_v2",
                    workspace_id = %self.workspace_id,
                    reason,
                    error = %err,
                    "generated task index rebuild failed; falling back to bundle scan",
                );
                Ok(false)
            }
        }
    }

    fn tasks_from_ids(&self, ids: Vec<String>) -> Result<Vec<Task>, OrbitError> {
        ids.into_iter()
            .map(|id| {
                let bundle = self.read_existing_bundle(&id)?;
                self.task_from_bundle(bundle)
            })
            .collect()
    }

    pub(super) fn replace_index_best_effort(&self, envelope: &TaskEnvelopeV2, operation: &str) {
        if let Err(err) = self
            .registry
            .replace_task_index(&self.workspace_id, envelope)
        {
            orbit_common::tracing::warn!(
                target: "orbit.store.task_v2",
                task_id = %envelope.id,
                workspace_id = %self.workspace_id,
                operation,
                error = %err,
                "task bundle was updated but generated task index update failed",
            );
        }
    }

    pub(super) fn task_from_bundle(&self, bundle: TaskBundleV2) -> Result<Task, OrbitError> {
        let status = bundle.envelope.status;
        Ok(Task {
            id: bundle.envelope.id,
            title: bundle.envelope.title,
            description: bundle.description,
            acceptance_criteria: parse_acceptance(&bundle.acceptance),
            tags: normalize_task_tags(bundle.envelope.tags),
            plan: bundle.plan,
            execution_summary: bundle.execution_summary,
            context_files: bundle.envelope.context_files,
            created_by: bundle.envelope.created_by,
            planned_by: bundle.envelope.planned_by,
            implemented_by: bundle.envelope.implemented_by,
            status,
            priority: bundle.envelope.priority,
            complexity: bundle.envelope.complexity,
            task_type: bundle.envelope.task_type,
            pr_status: None,
            external_refs: bundle.envelope.external_refs,
            relations: bundle.envelope.relations,
            job_run_id: bundle.envelope.job_run_id,
            crew: bundle.envelope.crew,
            created_at: bundle.envelope.created_at,
            updated_at: bundle.envelope.updated_at,
        })
    }

    pub(super) fn read_existing_bundle(&self, id: &str) -> Result<TaskBundleV2, OrbitError> {
        self.bundle_store.read_bundle(id).map_err(|err| match err {
            OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            } => OrbitError::not_found(NotFoundKind::Task, id.to_string()),
            other => other,
        })
    }

    pub(super) fn with_task_lock<T, F>(&self, id: &str, op: F) -> Result<T, OrbitError>
    where
        F: FnOnce() -> Result<T, OrbitError>,
    {
        let lock_target = self.bundle_store.bundle_path(id)?.join("task.yaml");
        with_exclusive_file_lock(&lock_target, "task artifact v2", op)
    }
}

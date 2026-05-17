use super::*;

impl TaskV2Store {
    pub(crate) fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if params.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let relations = relations_from_create_params(&params)?;

        let now = Utc::now();
        let id = self.registry.allocate_task_id(&self.workspace_id)?;
        let comments = params
            .comments
            .iter()
            .enumerate()
            .map(|(index, comment)| orbit_common::types::TaskCommentRowV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                comment_id: format!("C-{number:04}", number = index + 1),
                at: comment.at,
                by: comment.by.clone(),
                body: comment.message.clone(),
            })
            .collect();
        let bundle = TaskBundleV2 {
            envelope: orbit_common::types::TaskEnvelopeV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                id: id.clone(),
                title: params.title,
                status: params.status,
                task_type: params.task_type,
                priority: params.priority,
                complexity: params.complexity,
                job_run_id: None,
                crew: params.crew,
                relations,
                tags: normalize_task_tags(params.tags),
                context_files: params.context_files,
                external_refs: params.external_refs,
                created_by: params.created_by,
                planned_by: params.planned_by,
                implemented_by: params.implemented_by,
                created_at: now,
                updated_at: now,
            },
            description: params.description,
            acceptance: render_acceptance(&params.acceptance_criteria),
            plan: params.plan,
            execution_summary: params.execution_summary,
            events: vec![orbit_common::types::TaskEventRowV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                event_id: "EV-0001".to_string(),
                at: now,
                by: params.actor,
                event_type: "created".to_string(),
                note: None,
                from_status: None,
                to_status: Some(params.status),
            }],
            comments,
            review_threads: Vec::new(),
            artifact_manifest: None,
        };

        self.bundle_store.create_bundle(&bundle)?;
        self.replace_index_best_effort(&bundle.envelope, "task creation");
        self.task_from_bundle(bundle)
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        if let Some(tasks) = self.indexed_tasks(TaskIndexFilter {
            status: None,
            priority: None,
            job_run_id: None,
            tags: Vec::new(),
        })? {
            return Ok(tasks);
        }

        let mut tasks = self
            .bundle_store
            .list_bundles()?
            .into_iter()
            .map(|bundle| self.task_from_bundle(bundle))
            .collect::<Result<Vec<_>, _>>()?;
        sort_by_created_desc_id_asc(&mut tasks, |task| &task.created_at, |task| &task.id);
        Ok(tasks)
    }

    pub(crate) fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        let mut tasks = match self.indexed_tasks(TaskIndexFilter {
            status,
            priority,
            job_run_id: job_run_id.map(ToOwned::to_owned),
            tags: Vec::new(),
        })? {
            Some(tasks) => tasks,
            None => self.list_tasks()?,
        };
        tasks.retain(|task| {
            status.is_none_or(|value| task.status == value)
                && priority.is_none_or(|value| task.priority == value)
                && parent_id.is_none_or(|value| task.parent_id() == Some(value))
                && job_run_id.is_none_or(|value| task.job_run_id.as_deref() == Some(value))
                && external_ref.is_none_or(|value| {
                    task.external_refs.iter().any(|candidate| {
                        candidate.system == value.system && candidate.id == value.id
                    })
                })
                && has_external_ref_system.is_none_or(|value| {
                    task.external_refs
                        .iter()
                        .any(|candidate| candidate.system == value)
                })
        });
        Ok(tasks)
    }

    pub(crate) fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        let required_tags = normalize_task_tags(tags.to_vec());
        if required_tags.is_empty() {
            return self.list_tasks();
        }
        if let Some(tasks) = self.indexed_tasks(TaskIndexFilter {
            status: None,
            priority: None,
            job_run_id: None,
            tags: required_tags.clone(),
        })? {
            return Ok(tasks);
        }
        let mut tasks = self.list_tasks()?;
        tasks.retain(|task| {
            required_tags
                .iter()
                .all(|required| task.tags.iter().any(|tag| tag == required))
        });
        Ok(tasks)
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        match self.bundle_store.read_bundle(id) {
            Ok(bundle) => self.task_from_bundle(bundle).map(Some),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks_filtered(query, &[])
    }

    pub(crate) fn search_tasks_filtered(
        &self,
        query: &str,
        tags: &[String],
    ) -> Result<Vec<Task>, OrbitError> {
        let lowered = query.to_lowercase();
        let mut tasks = self.list_tasks_by_tags(tags)?;
        let mut matches = Vec::new();
        for task in tasks.drain(..) {
            if self.task_matches_query(&task, &lowered)? {
                matches.push(task);
            }
        }
        Ok(matches)
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        let lock_target = self.bundle_store.bundle_path(id)?;
        with_exclusive_file_lock(&lock_target, "task artifact v2 delete", || {
            self.bundle_store.delete_bundle(id)
        })
    }
}

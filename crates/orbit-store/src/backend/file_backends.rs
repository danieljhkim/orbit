use chrono::{DateTime, Utc};
use orbit_common::types::{
    Adr, AdrStatus, ArtifactManifestFileV2, Crew, ExecutorDef, ExternalRef, JobRun,
    KnowledgeRunMetrics, Learning, LearningStatus, OrbitError, PipelineState, PolicyDef,
    ReviewThread, Task, TaskArtifact, TaskComment, TaskHistoryEntry, TaskPriority, TaskStatus,
};

use super::contracts::{
    AdrCreateParams, AdrDocumentUpdateParams, AdrListEntry, AdrStoreBackend,
    ExecutorDefStoreBackend, JobRunQuery, JobRunStepParams, JobRunStoreBackend,
    LearningCommentAddParams, LearningCommentDeleteParams, LearningCreateParams, LearningListEntry,
    LearningSearchParams, LearningSearchResult, LearningStoreBackend, LearningUpdateParams,
    LearningUpvoteParams, PolicyDefStoreBackend, RemoteArtifactStub, TaskArtifactStoreBackend,
    TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentStoreBackend, TaskDocumentUpdateParams,
    TaskHistoryStoreBackend, TaskHistoryUpdateParams, TaskReviewStoreBackend,
    TaskReviewUpdateParams, TaskStoreBackend,
};
use crate::file::adr_store::AdrFileStore;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::learning_store::LearningFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskV2Store;
use crate::scope::{ScopeStrategy, ScopedStore, resolve};

impl TaskStoreBackend for TaskV2Store {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.create_task(params)
    }

    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks()
    }

    fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks_by_tags(tags)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks_filtered(
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        resolve::<Task, _>(self, id)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks(query)
    }

    fn search_tasks_filtered(&self, query: &str, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks_filtered(query, tags)
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_task(id)
    }
}

impl ScopedStore<Task> for TaskV2Store {
    type Err = OrbitError;

    fn strategy(&self) -> ScopeStrategy {
        ScopeStrategy::WorkspaceOnly
    }

    fn get_workspace(&self, key: &str) -> Result<Option<Task>, OrbitError> {
        self.get_task(key)
    }

    fn get_global(&self, _key: &str) -> Result<Option<Task>, OrbitError> {
        Ok(None)
    }
}

impl TaskDocumentStoreBackend for TaskV2Store {
    fn update_task_document(
        &self,
        id: &str,
        params: TaskDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_document(id, &params)
    }
}

impl TaskHistoryStoreBackend for TaskV2Store {
    fn get_task_comments(&self, id: &str) -> Result<Option<Vec<TaskComment>>, OrbitError> {
        self.get_task_comments(id)
    }

    fn get_task_history(&self, id: &str) -> Result<Option<Vec<TaskHistoryEntry>>, OrbitError> {
        self.get_task_history(id)
    }

    fn update_task_history(
        &self,
        id: &str,
        params: TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_history(id, &params)
    }
}

impl TaskReviewStoreBackend for TaskV2Store {
    fn get_task_review_threads(&self, id: &str) -> Result<Option<Vec<ReviewThread>>, OrbitError> {
        self.get_task_review_threads(id)
    }

    fn update_task_reviews(
        &self,
        id: &str,
        params: TaskReviewUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_reviews(id, &params)
    }
}

impl TaskArtifactStoreBackend for TaskV2Store {
    fn get_task_artifact_manifest(
        &self,
        id: &str,
    ) -> Result<Option<Vec<ArtifactManifestFileV2>>, OrbitError> {
        self.get_task_artifact_manifest(id)
    }

    fn get_task_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        self.get_task_artifacts(id)
    }

    fn get_task_artifact(&self, id: &str, path: &str) -> Result<Option<TaskArtifact>, OrbitError> {
        self.get_task_artifact(id, path)
    }

    fn upsert_task_artifacts(
        &self,
        id: &str,
        params: TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError> {
        self.upsert_task_artifacts(id, &params)
    }
}

impl JobRunStoreBackend for JobFileStore {
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs_filtered(query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_job_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_pending_or_running_job_runs(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.insert_job_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.mark_job_run_running(run_id, started_at, pid)
    }

    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.take_over_running_job_run(
            run_id,
            expected_pid,
            expected_pid_start_time,
            started_at,
            pid,
        )
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.abandon_job_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.complete_job_run_step(run_id, params)
    }

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.record_job_run_knowledge_metrics(run_id, metrics)
    }

    fn record_job_run_crew(&self, run_id: &str, crew: &Crew) -> Result<bool, OrbitError> {
        self.record_job_run_crew(run_id, crew)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: orbit_common::types::JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn repair_terminal_job_run_timing(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.repair_terminal_job_run_timing(run_id, finished_at, duration_ms)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.archive_run(run_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.delete_run(run_id)
    }

    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError> {
        self.read_run_state(run_id)
    }

    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError> {
        self.write_run_state(run_id, state)
    }

    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.list_all_pending_or_running_runs()
    }
}

impl ExecutorDefStoreBackend for ExecutorDefFileStore {
    fn list_executor_defs(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        self.list_executor_defs()
    }

    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.get_executor_def(name)
    }

    fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        self.upsert_executor_def(def)
    }
}

impl PolicyDefStoreBackend for PolicyDefFileStore {
    fn list_policy_defs(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        self.list_policy_defs()
    }

    fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        self.get_policy_def(name)
    }

    fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.upsert_policy_def(def)
    }
}

impl AdrStoreBackend for AdrFileStore {
    fn add_adr(&self, params: AdrCreateParams) -> Result<Adr, OrbitError> {
        self.add_adr(params)
    }

    fn get_adr(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        // ADRs use the WorkspaceOnly strategy per `CLAUDE.md`.
        resolve::<Adr, _>(self, id)
    }

    fn get_adr_federated(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        AdrFileStore::get_adr_federated(self, id)
    }

    fn list_adrs(&self) -> Result<Vec<Adr>, OrbitError> {
        self.list_adrs()
    }

    fn list_adrs_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        validation_warned: Option<bool>,
    ) -> Result<Vec<Adr>, OrbitError> {
        self.list_adrs_filtered(
            status,
            owner,
            feature,
            task_id,
            legacy_id,
            validation_warned,
        )
    }

    fn list_adr_entries_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        validation_warned: Option<bool>,
        include_remote: bool,
    ) -> Result<Vec<AdrListEntry>, OrbitError> {
        AdrFileStore::list_adr_entries_filtered(
            self,
            status,
            owner,
            feature,
            task_id,
            legacy_id,
            validation_warned,
            include_remote,
        )
    }

    fn get_adr_remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        AdrFileStore::get_adr_remote_stub(self, id)
    }

    fn update_adr_status(&self, id: &str, new_status: AdrStatus) -> Result<(), OrbitError> {
        self.update_adr_status(id, new_status)
    }

    fn update_adr_document(
        &self,
        id: &str,
        fields: &AdrDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_adr_document(id, fields)
    }

    fn delete_adr(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_adr(id)
    }

    fn rebuild_index(&self) -> Result<(), OrbitError> {
        self.rebuild_index()
    }

    fn supersede_adr(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        self.supersede_adr(old_id, new_id)
    }
}

impl LearningStoreBackend for LearningFileStore {
    fn create_learning(&self, params: LearningCreateParams) -> Result<Learning, OrbitError> {
        self.create_learning(params)
    }

    fn get_learning(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        // Learnings use the WorkspaceOnly strategy per `CLAUDE.md` Scoping
        // Rules and ADR-003.
        resolve::<Learning, _>(self, id)
    }

    fn get_learning_federated(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        LearningFileStore::get_learning_federated(self, id)
    }

    fn list_learnings(&self, status: Option<LearningStatus>) -> Result<Vec<Learning>, OrbitError> {
        self.list_learnings(status)
    }

    fn list_learning_entries(
        &self,
        status: Option<LearningStatus>,
        include_remote: bool,
    ) -> Result<Vec<LearningListEntry>, OrbitError> {
        LearningFileStore::list_learning_entries(self, status, include_remote)
    }

    fn get_learning_remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        LearningFileStore::get_learning_remote_stub(self, id)
    }

    fn search_learnings(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError> {
        self.search_learnings(params)
    }

    fn upvote_learning(
        &self,
        params: LearningUpvoteParams,
    ) -> Result<orbit_common::types::LearningVoteSummary, OrbitError> {
        self.upvote_learning(params)
    }

    fn learning_vote_summary(
        &self,
        id: &str,
    ) -> Result<orbit_common::types::LearningVoteSummary, OrbitError> {
        self.learning_vote_summary(id)
    }

    fn add_learning_comment(
        &self,
        params: LearningCommentAddParams,
    ) -> Result<orbit_common::types::LearningComment, OrbitError> {
        self.add_learning_comment(params)
    }

    fn list_learning_comments(
        &self,
        learning_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<orbit_common::types::LearningComment>, OrbitError> {
        self.list_learning_comments(learning_id, include_deleted)
    }

    fn delete_learning_comment(
        &self,
        params: LearningCommentDeleteParams,
    ) -> Result<(), OrbitError> {
        self.delete_learning_comment(params)
    }

    fn update_learning(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError> {
        self.update_learning(id, params)
    }

    fn supersede_learning(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        self.supersede_learning(old_id, new_id)
    }

    fn archive_learning(&self, id: &str) -> Result<bool, OrbitError> {
        self.archive_learning(id)
    }

    fn delete_learning(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_learning(id)
    }

    fn reindex_learnings(&self) -> Result<(), OrbitError> {
        self.reindex_learnings()
    }
}

impl ScopedStore<Learning> for LearningFileStore {
    type Err = OrbitError;

    fn strategy(&self) -> ScopeStrategy {
        ScopeStrategy::WorkspaceOnly
    }

    fn get_workspace(&self, key: &str) -> Result<Option<Learning>, OrbitError> {
        self.get_learning(key)
    }

    fn get_global(&self, _key: &str) -> Result<Option<Learning>, OrbitError> {
        Ok(None)
    }
}

impl ScopedStore<Adr> for AdrFileStore {
    type Err = OrbitError;

    fn strategy(&self) -> ScopeStrategy {
        ScopeStrategy::WorkspaceOnly
    }

    fn get_workspace(&self, key: &str) -> Result<Option<Adr>, OrbitError> {
        self.get_adr(key)
    }

    fn get_global(&self, _key: &str) -> Result<Option<Adr>, OrbitError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn adr_file_store_returns_workspace_only_strategy() {
        let store = AdrFileStore::new(PathBuf::from("/tmp/unused-adr-root"));
        assert_eq!(
            ScopedStore::<Adr>::strategy(&store),
            ScopeStrategy::WorkspaceOnly
        );
    }
}

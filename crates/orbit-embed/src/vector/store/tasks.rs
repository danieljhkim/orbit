//! Task-corpus indexing entry points.
//!
//! `index_task` and `reindex_tasks` are the convenience wrappers that wire
//! `task_embedding_fields(...)` (per-field extraction) into `upsert_embeddings`.

use orbit_common::types::{OrbitError, Task};

use super::{SOURCE_KIND_TASK, VectorStore};
use crate::Embedder;
use crate::vector::UpsertReport;
use crate::vector::task_fields::task_embedding_fields;

impl VectorStore {
    pub fn index_task(
        &self,
        task: &Task,
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        self.upsert_embeddings(
            SOURCE_KIND_TASK,
            &task.id,
            &task_embedding_fields(task),
            embedder,
            force,
        )
    }

    pub fn reindex_tasks(
        &self,
        tasks: &[Task],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        let mut total = UpsertReport::default();
        for task in tasks {
            let report = self.index_task(task, embedder, force)?;
            total.embedded_chunks += report.embedded_chunks;
            total.skipped_fields += report.skipped_fields;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{TaskPriority, TaskStatus, TaskType};

    use super::*;
    use crate::NoopEmbedder;

    fn task(id: &str, title: &str, description: &str) -> Task {
        Task {
            id: id.to_string(),
            parent_id: None,
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: vec!["First criterion".to_string()],
            dependencies: Vec::new(),
            plan: "Plan body".to_string(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            workspace_path: None,
            repo_root: None,
            created_by: None,
            planned_by: None,
            implemented_by: None,
            agent: None,
            model: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            pr_status: None,
            external_refs: Vec::new(),
            source_task_id: None,
            batch_id: None,
            tags: Vec::new(),
            comments: Vec::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn noop_task_indexing_populates_rows_without_companion() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = NoopEmbedder::small();
        let task = task("T1", "Index this", "Task description");

        let report = store.index_task(&task, &embedder, false).unwrap();
        let stats = store.stats(&["T1".to_string()]).unwrap();

        assert!(report.embedded_chunks >= 3);
        assert_eq!(stats.stale_rows, 0);
        assert_eq!(stats.counts[0].source_kind, "task");
        assert_eq!(stats.counts[0].model_id, "noop");
    }
}

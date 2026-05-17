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
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: vec!["First criterion".to_string()],
            plan: "Plan body".to_string(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            tags: Vec::new(),
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

    #[test]
    fn reindex_tasks_removes_legacy_field_rows_after_rename() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = NoopEmbedder::small();
        store
            .upsert_embeddings(
                SOURCE_KIND_TASK,
                "T1",
                &[
                    crate::vector::EmbeddingField::new("purpose", "old purpose"),
                    crate::vector::EmbeddingField::new("summary", "old summary"),
                    crate::vector::EmbeddingField::new("acceptance_criteria", "old acceptance"),
                ],
                &embedder,
                false,
            )
            .expect("prepopulate legacy rows");

        store
            .reindex_tasks(
                &[task("T1", "New title", "New description")],
                &embedder,
                false,
            )
            .expect("reindex task");

        let conn = store.connection();
        let conn = conn.lock().unwrap();
        let embeddings: i64 = conn
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM embeddings
                    WHERE source_id = 'T1'
                      AND field IN ('purpose', 'summary', 'acceptance_criteria')
                "#,
                [],
                |row| row.get(0),
            )
            .unwrap();
        let fts: i64 = conn
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM tasks_fts
                    WHERE source_id = 'T1'
                      AND field IN ('purpose', 'summary', 'acceptance_criteria')
                "#,
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(embeddings, 0);
        assert_eq!(fts, 0);
    }
}

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

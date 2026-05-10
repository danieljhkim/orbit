use std::collections::BTreeSet;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;

use orbit_common::types::{OrbitError, Task};
use orbit_embed::{Embedder, SubprocessEmbedder};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{Store, now_string};

const SOURCE_KIND_TASK: &str = "task";
const TARGET_CHUNK_TOKENS: usize = 400;
const OVERLAP_TOKENS: usize = 50;
const EMBED_BATCH_SIZE: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingField {
    pub field: String,
    pub text: String,
}

impl EmbeddingField {
    pub fn new(field: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertReport {
    pub embedded_chunks: usize,
    pub skipped_fields: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceModelCount {
    pub source_kind: String,
    pub model_id: String,
    pub rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticStats {
    pub counts: Vec<SourceModelCount>,
    pub stale_rows: usize,
}

#[derive(Clone)]
pub struct VectorStore {
    store: Store,
}

impl VectorStore {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn upsert_embeddings(
        &self,
        source_kind: &str,
        source_id: &str,
        fields: &[EmbeddingField],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        let mut report = UpsertReport::default();
        let conn = self.store.connection();
        let mut conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        let tx = conn
            .transaction()
            .map_err(|error| OrbitError::Store(error.to_string()))?;

        for field in fields {
            if field.text.trim().is_empty() {
                delete_field_rows(
                    &tx,
                    source_kind,
                    source_id,
                    &field.field,
                    embedder.model_id(),
                )?;
                continue;
            }
            let field_hash = content_hash(&field.text);
            if !force
                && field_content_hash_unchanged(
                    &tx,
                    source_kind,
                    source_id,
                    &field.field,
                    embedder.model_id(),
                    &field_hash,
                )?
            {
                report.skipped_fields += 1;
                continue;
            }
            let chunks = chunk_text(
                &field.text,
                embedder,
                TARGET_CHUNK_TOKENS.min(embedder.max_input_tokens()),
                OVERLAP_TOKENS,
            )?;
            let hashes = vec![field_hash; chunks.len()];

            delete_field_rows(
                &tx,
                source_kind,
                source_id,
                &field.field,
                embedder.model_id(),
            )?;
            let text_refs = chunks.iter().map(String::as_str).collect::<Vec<_>>();
            let vectors = embedder.embed(&text_refs)?;
            if vectors.len() != chunks.len() {
                return Err(OrbitError::Execution(format!(
                    "embedder returned {} vectors for {} chunks",
                    vectors.len(),
                    chunks.len()
                )));
            }
            for (idx, ((chunk, hash), vector)) in chunks
                .iter()
                .zip(hashes.iter())
                .zip(vectors.iter())
                .enumerate()
            {
                if vector.len() != embedder.dim() {
                    return Err(OrbitError::Execution(format!(
                        "embedder returned dim {} but advertised {}",
                        vector.len(),
                        embedder.dim()
                    )));
                }
                tx.execute(
                    r#"
                        INSERT INTO embeddings(
                            source_kind, source_id, field, chunk_idx, content_hash,
                            model_id, dim, embedding, created_at
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                        ON CONFLICT(source_kind, source_id, field, chunk_idx, model_id)
                        DO UPDATE SET
                            content_hash = excluded.content_hash,
                            dim = excluded.dim,
                            embedding = excluded.embedding,
                            created_at = excluded.created_at
                    "#,
                    params![
                        source_kind,
                        source_id,
                        field.field,
                        idx as i64,
                        hash,
                        embedder.model_id(),
                        embedder.dim() as i64,
                        encode_f32_blob(vector),
                        now_string(),
                    ],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
                if source_kind == SOURCE_KIND_TASK {
                    tx.execute(
                        "INSERT INTO tasks_fts(source_id, field, content) VALUES (?1, ?2, ?3)",
                        params![source_id, field.field, chunk],
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                }
                report.embedded_chunks += 1;
            }
        }

        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(report)
    }

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

    pub fn delete_source(&self, source_kind: &str, source_id: &str) -> Result<(), OrbitError> {
        let conn = self.store.connection();
        let conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        conn.execute(
            "DELETE FROM embeddings WHERE source_kind = ?1 AND source_id = ?2",
            params![source_kind, source_id],
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
        if source_kind == SOURCE_KIND_TASK {
            conn.execute(
                "DELETE FROM tasks_fts WHERE source_id = ?1",
                params![source_id],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        }
        Ok(())
    }

    pub fn stats(&self, current_task_ids: &[String]) -> Result<SemanticStats, OrbitError> {
        let conn = self.store.connection();
        let conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT source_kind, model_id, COUNT(*)
                    FROM embeddings
                    GROUP BY source_kind, model_id
                    ORDER BY source_kind, model_id
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SourceModelCount {
                    source_kind: row.get(0)?,
                    model_id: row.get(1)?,
                    rows: row.get::<_, i64>(2)? as usize,
                })
            })
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }

        let current = current_task_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut stmt = conn
            .prepare("SELECT DISTINCT source_id FROM embeddings WHERE source_kind = 'task'")
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut stale_rows = 0;
        for row in rows {
            let source_id = row.map_err(|error| OrbitError::Store(error.to_string()))?;
            if !current.contains(&source_id) {
                stale_rows += 1;
            }
        }
        Ok(SemanticStats { counts, stale_rows })
    }
}

#[derive(Debug, Clone)]
pub struct EmbedJob {
    pub task: Task,
    pub force: bool,
}

#[derive(Clone)]
pub struct EmbedWorker {
    sender: SyncSender<EmbedJob>,
}

impl EmbedWorker {
    pub fn start(store: VectorStore) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<EmbedJob>(128);
        thread::spawn(move || {
            let mut embedder: Option<SubprocessEmbedder> = None;
            while let Ok(first) = receiver.recv() {
                let mut batch = vec![first];
                while batch.len() < EMBED_BATCH_SIZE {
                    match receiver.try_recv() {
                        Ok(job) => batch.push(job),
                        Err(_) => break,
                    }
                }
                if embedder.is_none() {
                    match SubprocessEmbedder::new() {
                        Ok(value) => embedder = Some(value),
                        Err(error) => {
                            orbit_common::tracing::debug!(
                                target: "orbit.semantic.indexer",
                                error = %error,
                                "semantic indexing skipped because embedder initialization failed",
                            );
                            continue;
                        }
                    }
                }
                let Some(active_embedder) = embedder.as_ref() else {
                    continue;
                };
                for job in &batch {
                    if let Err(error) = store.index_task(&job.task, active_embedder, job.force) {
                        orbit_common::tracing::debug!(
                            target: "orbit.semantic.indexer",
                            task_id = job.task.id.as_str(),
                            error = %error,
                            "semantic indexing failed after task mutation",
                        );
                    }
                }
            }
        });
        Self { sender }
    }

    pub fn enqueue(&self, task: Task) {
        match self.sender.try_send(EmbedJob { task, force: false }) {
            Ok(()) => {}
            Err(TrySendError::Full(job)) => {
                orbit_common::tracing::debug!(
                    target: "orbit.semantic.indexer",
                    task_id = job.task.id.as_str(),
                    "semantic indexing queue is full; dropping task update",
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                orbit_common::tracing::debug!(
                    target: "orbit.semantic.indexer",
                    "semantic indexing queue is disconnected; dropping task update",
                );
            }
        }
    }
}

pub fn task_embedding_fields(task: &Task) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "purpose", &task.title);
    push_field(&mut fields, "summary", &task.description);
    push_field(&mut fields, "plan", &task.plan);
    push_field(&mut fields, "execution_summary", &task.execution_summary);
    if !task.acceptance_criteria.is_empty() {
        push_field(
            &mut fields,
            "acceptance_criteria",
            &task.acceptance_criteria.join("\n"),
        );
    }
    for (idx, comment) in task.comments.iter().enumerate() {
        push_field(&mut fields, format!("comment_{idx}"), &comment.message);
    }
    for thread in &task.review_threads {
        for (idx, message) in thread.messages.iter().enumerate() {
            push_field(
                &mut fields,
                format!("review_{}_msg_{idx}", thread.thread_id),
                &message.body,
            );
        }
    }
    fields
}

fn push_field(fields: &mut Vec<EmbeddingField>, field: impl Into<String>, text: &str) {
    if !text.trim().is_empty() {
        fields.push(EmbeddingField::new(field, text.trim().to_string()));
    }
}

pub fn chunk_text(
    text: &str,
    embedder: &dyn Embedder,
    target_tokens: usize,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    let target_tokens = target_tokens.max(1);
    if embedder.token_count(text)? <= target_tokens {
        return Ok(vec![text.trim().to_string()]);
    }

    let paragraphs = split_paragraphs(text);
    let mut chunks = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_tokens = 0;

    for paragraph in paragraphs {
        let paragraph_tokens = embedder.token_count(&paragraph)?;
        if paragraph_tokens > target_tokens {
            if !current.is_empty() {
                chunks.push(current.join("\n\n"));
                current = overlap_tail(&current, embedder, overlap_tokens)?;
                current_tokens = count_parts(&current, embedder)?;
            }
            for piece in split_long_paragraph(&paragraph, embedder, target_tokens, overlap_tokens)?
            {
                chunks.push(piece);
            }
            current.clear();
            continue;
        }

        if !current.is_empty() && current_tokens + paragraph_tokens > target_tokens {
            chunks.push(current.join("\n\n"));
            current = overlap_tail(&current, embedder, overlap_tokens)?;
            current_tokens = count_parts(&current, embedder)?;
        }
        current.push(paragraph);
        current_tokens += paragraph_tokens;
    }

    if !current.is_empty() {
        chunks.push(current.join("\n\n"));
    }
    Ok(chunks)
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.trim().to_string());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs
}

fn overlap_tail(
    paragraphs: &[String],
    embedder: &dyn Embedder,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    if overlap_tokens == 0 {
        return Ok(Vec::new());
    }
    let mut selected = Vec::new();
    let mut total = 0;
    for paragraph in paragraphs.iter().rev() {
        let tokens = embedder.token_count(paragraph)?;
        if total > 0 && total + tokens > overlap_tokens {
            break;
        }
        selected.push(paragraph.clone());
        total += tokens;
        if total >= overlap_tokens {
            break;
        }
    }
    selected.reverse();
    Ok(selected)
}

fn count_parts(parts: &[String], embedder: &dyn Embedder) -> Result<usize, OrbitError> {
    parts
        .iter()
        .try_fold(0, |sum, part| Ok(sum + embedder.token_count(part)?))
}

fn split_long_paragraph(
    paragraph: &str,
    embedder: &dyn Embedder,
    target_tokens: usize,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    let words = paragraph.split_whitespace().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let mut end = start + 1;
        while end <= words.len() {
            let candidate = words[start..end].join(" ");
            if embedder.token_count(&candidate)? > target_tokens {
                break;
            }
            end += 1;
        }
        let chunk_end = (end - 1).max(start + 1).min(words.len());
        chunks.push(words[start..chunk_end].join(" "));
        if chunk_end == words.len() {
            break;
        }
        let mut overlap_start = chunk_end;
        while overlap_start > start {
            let candidate = words[overlap_start - 1..chunk_end].join(" ");
            if embedder.token_count(&candidate)? > overlap_tokens {
                break;
            }
            overlap_start -= 1;
        }
        start = overlap_start.max(start + 1);
    }
    Ok(chunks)
}

fn field_content_hash_unchanged(
    conn: &Connection,
    source_kind: &str,
    source_id: &str,
    field: &str,
    model_id: &str,
    expected_hash: &str,
) -> Result<bool, OrbitError> {
    let mut stmt = conn
        .prepare(
            r#"
                SELECT content_hash
                FROM embeddings
                WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3 AND model_id = ?4
            "#,
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(params![source_kind, source_id, field, model_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let mut found = false;
    for row in rows {
        found = true;
        let hash = row.map_err(|error| OrbitError::Store(error.to_string()))?;
        if hash != expected_hash {
            return Ok(false);
        }
    }
    Ok(found)
}

fn delete_field_rows(
    conn: &Connection,
    source_kind: &str,
    source_id: &str,
    field: &str,
    model_id: &str,
) -> Result<(), OrbitError> {
    conn.execute(
        r#"
            DELETE FROM embeddings
            WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3 AND model_id = ?4
        "#,
        params![source_kind, source_id, field, model_id],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    if source_kind == SOURCE_KIND_TASK {
        conn.execute(
            "DELETE FROM tasks_fts WHERE source_id = ?1 AND field = ?2",
            params![source_id, field],
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    }
    Ok(())
}

fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

pub fn encode_f32_blob(values: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(values.len() * 4);
    for value in values {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

pub fn decode_f32_blob(blob: &[u8]) -> Result<Vec<f32>, OrbitError> {
    if !blob.len().is_multiple_of(4) {
        return Err(OrbitError::Store(format!(
            "invalid embedding blob length {}; expected multiple of 4",
            blob.len()
        )));
    }
    Ok(blob
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32, OrbitError> {
    if left.len() != right.len() {
        return Err(OrbitError::InvalidInput(format!(
            "vector length mismatch: {} != {}",
            left.len(),
            right.len()
        )));
    }
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    let denom = left_norm.sqrt() * right_norm.sqrt();
    if denom == 0.0 {
        return Ok(0.0);
    }
    Ok(dot / denom)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
    use orbit_embed::NoopEmbedder;

    use super::*;

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
            comments: Vec::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn cosine_helper_scores_toy_vectors() {
        assert!(
            (cosine_similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]).unwrap() - 1.0).abs() < 0.0001
        );
        assert!(
            cosine_similarity(&[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0])
                .unwrap()
                .abs()
                < 0.0001
        );
        assert!(cosine_similarity(&[1.0, 0.0, 0.0], &[-1.0, 0.0, 0.0]).unwrap() < -0.999);
    }

    #[test]
    fn upsert_embeddings_skips_unchanged_content_hashes() {
        let store = VectorStore::new(Store::open_in_memory().unwrap());
        let embedder = NoopEmbedder::small();
        let fields = vec![EmbeddingField::new("purpose", "same content")];

        let first = store
            .upsert_embeddings("task", "T1", &fields, &embedder, false)
            .unwrap();
        let second = store
            .upsert_embeddings("task", "T1", &fields, &embedder, false)
            .unwrap();

        assert_eq!(first.embedded_chunks, 1);
        assert_eq!(second.embedded_chunks, 0);
        assert_eq!(second.skipped_fields, 1);
    }

    #[test]
    fn paragraph_chunker_overlaps_at_boundaries() {
        let embedder = NoopEmbedder::new("noop", 3, 64);
        let text = "one two three\n\nfour five six\n\nseven eight nine";
        let chunks = chunk_text(text, &embedder, 5, 3).unwrap();

        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].contains("one two three"));
        assert!(chunks[1].contains("one two three"));
        assert!(chunks[1].contains("four five six"));
        assert!(chunks[2].contains("four five six"));
    }

    #[test]
    fn delete_source_cascades_vector_and_fts_rows() {
        let store = VectorStore::new(Store::open_in_memory().unwrap());
        let embedder = NoopEmbedder::small();
        store
            .upsert_embeddings(
                "task",
                "T1",
                &[EmbeddingField::new("purpose", "delete me")],
                &embedder,
                false,
            )
            .unwrap();

        store.delete_source("task", "T1").unwrap();

        let conn = store.store.connection();
        let conn = conn.lock().unwrap();
        let embeddings: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap();
        let fts: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(embeddings, 0);
        assert_eq!(fts, 0);
    }

    #[test]
    fn noop_task_indexing_populates_rows_without_companion() {
        let store = VectorStore::new(Store::open_in_memory().unwrap());
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

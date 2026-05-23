//! BLAKE3-deduped per-field write path.
//!
//! `upsert_embeddings` is the canonical entry: it transactionally chunks each
//! field's text, embeds the chunks, and writes the resulting rows into both
//! `embeddings` (vector storage) and `corpus_fts` (FTS5 lexical mirror).
//! Unchanged fields short-circuit via `content_hash`.

use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::OrbitError;
use rusqlite::{Connection, params};

use super::VectorStore;
use crate::Embedder;
use crate::vector::chunker::chunk_text;
use crate::vector::{EmbeddingField, UpsertReport, encode_f32_blob};

const TARGET_CHUNK_TOKENS: usize = 400;
const OVERLAP_TOKENS: usize = 50;

impl VectorStore {
    /// Replace the indexed field set for a source.
    ///
    /// `fields` is the complete current field set, not a partial patch; rows
    /// for previously indexed fields absent from this slice are removed.
    pub fn upsert_embeddings(
        &self,
        source_kind: &str,
        source_id: &str,
        fields: &[EmbeddingField],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        let mut report = UpsertReport::default();
        let conn = self.connection();
        let mut conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        let tx = conn
            .transaction()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let expected_fields = fields
            .iter()
            .map(|field| field.field.as_str())
            .collect::<BTreeSet<_>>();
        delete_unexpected_field_rows(&tx, source_kind, source_id, &expected_fields)?;

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
                tx.execute(
                    "INSERT INTO corpus_fts(source_kind, source_id, field, content) VALUES (?1, ?2, ?3, ?4)",
                    params![source_kind, source_id, field.field, chunk],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
                report.embedded_chunks += 1;
            }
        }

        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(report)
    }
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

pub(super) fn delete_field_rows(
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
    conn.execute(
        "DELETE FROM corpus_fts WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3",
        params![source_kind, source_id, field],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn delete_unexpected_field_rows(
    conn: &Connection,
    source_kind: &str,
    source_id: &str,
    expected_fields: &BTreeSet<&str>,
) -> Result<(), OrbitError> {
    let mut stored_fields = BTreeSet::new();
    {
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT DISTINCT field
                    FROM embeddings
                    WHERE source_kind = ?1 AND source_id = ?2
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map(params![source_kind, source_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        for row in rows {
            stored_fields.insert(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }
    }
    {
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT DISTINCT field
                    FROM corpus_fts
                    WHERE source_kind = ?1 AND source_id = ?2
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map(params![source_kind, source_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        for row in rows {
            stored_fields.insert(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }
    }

    for field in stored_fields {
        if !expected_fields.contains(field.as_str()) {
            delete_source_field_rows(conn, source_kind, source_id, &field)?;
        }
    }
    Ok(())
}

fn delete_source_field_rows(
    conn: &Connection,
    source_kind: &str,
    source_id: &str,
    field: &str,
) -> Result<(), OrbitError> {
    conn.execute(
        r#"
            DELETE FROM embeddings
            WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3
        "#,
        params![source_kind, source_id, field],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    conn.execute(
        "DELETE FROM corpus_fts WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3",
        params![source_kind, source_id, field],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

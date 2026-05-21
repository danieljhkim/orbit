use orbit_common::types::OrbitError;
use rusqlite::params;

use crate::vector::store::VectorStore;

#[derive(Debug, Clone, PartialEq)]
pub struct Bm25Hit {
    pub source_kind: String,
    pub source_id: String,
    pub field: String,
    pub rowid: i64,
    pub rank: usize,
}

pub fn bm25_top_k(
    store: &VectorStore,
    query: &str,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<Bm25Hit>, OrbitError> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let match_query = fts_phrase_quote(query);
    let conn = store.connection();
    let conn = conn
        .lock()
        .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
    let mut hits = Vec::new();
    if let Some(kind) = kind {
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT source_kind, source_id, field, rowid, bm25(corpus_fts) AS rank
                    FROM corpus_fts
                    WHERE corpus_fts MATCH ?1 AND source_kind = ?2
                    ORDER BY rank
                    LIMIT ?3
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut rows = stmt
            .query(params![match_query, kind, limit as i64])
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        collect_hits(&mut rows, &mut hits)?;
    } else {
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT source_kind, source_id, field, rowid, bm25(corpus_fts) AS rank
                    FROM corpus_fts
                    WHERE corpus_fts MATCH ?1
                    ORDER BY rank
                    LIMIT ?2
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut rows = stmt
            .query(params![match_query, limit as i64])
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        collect_hits(&mut rows, &mut hits)?;
    }
    Ok(hits)
}

fn collect_hits(rows: &mut rusqlite::Rows<'_>, hits: &mut Vec<Bm25Hit>) -> Result<(), OrbitError> {
    while let Some(row) = rows
        .next()
        .map_err(|error| OrbitError::Store(error.to_string()))?
    {
        hits.push(Bm25Hit {
            source_kind: row
                .get(0)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            source_id: row
                .get(1)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            field: row
                .get(2)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            rowid: row
                .get(3)
                .map_err(|error| OrbitError::Store(error.to_string()))?,
            rank: hits.len() + 1,
        });
    }
    Ok(())
}

pub fn snippet_for_hit(
    store: &VectorStore,
    source_kind: &str,
    source_id: &str,
    field: &str,
    chunk_idx: Option<usize>,
    rowid: Option<i64>,
) -> Result<Option<String>, OrbitError> {
    if let Some(rowid) = rowid {
        return snippet_by_rowid(store, rowid);
    }
    let Some(chunk_idx) = chunk_idx else {
        return Ok(None);
    };
    snippet_by_chunk_idx(store, source_kind, source_id, field, chunk_idx)
}

fn snippet_by_rowid(store: &VectorStore, rowid: i64) -> Result<Option<String>, OrbitError> {
    let conn = store.connection();
    let conn = conn
        .lock()
        .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
    conn.query_row(
        "SELECT content FROM corpus_fts WHERE rowid = ?1",
        params![rowid],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(OrbitError::Store(other.to_string())),
    })
}

fn snippet_by_chunk_idx(
    store: &VectorStore,
    source_kind: &str,
    source_id: &str,
    field: &str,
    chunk_idx: usize,
) -> Result<Option<String>, OrbitError> {
    let conn = store.connection();
    let conn = conn
        .lock()
        .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
    conn.query_row(
        r#"
            SELECT content
            FROM corpus_fts
            WHERE source_kind = ?1 AND source_id = ?2 AND field = ?3
            ORDER BY rowid
            LIMIT 1 OFFSET ?4
        "#,
        params![source_kind, source_id, field, chunk_idx as i64],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(OrbitError::Store(other.to_string())),
    })
}

// widened for tests per ORB-00230 sibling layout; see test_layout.md
pub(crate) fn fts_phrase_quote(query: &str) -> String {
    format!("\"{}\"", query.trim().replace('"', "\"\""))
}

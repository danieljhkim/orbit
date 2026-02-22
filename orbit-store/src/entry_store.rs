use chrono::Utc;
use orbit_types::{AuthorType, EntityType, Entry, EntryType, OrbitError};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx, new_id, parse_timestamp};

impl Store {
    pub fn list_entries(
        &self,
        entity_type: EntityType,
        entity_id: &str,
    ) -> Result<Vec<Entry>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, entity_type, entity_id, session_id, sequence_number, entry_type, author_type, author_id, author_model, body, created_at
                 FROM entries
                 WHERE entity_type = ?1 AND entity_id = ?2
                 ORDER BY sequence_number ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![entity_type.to_string(), entity_id], row_to_entry)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_entries_by_session(&self, session_id: &str) -> Result<Vec<Entry>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, entity_type, entity_id, session_id, sequence_number, entry_type, author_type, author_id, author_model, body, created_at
                 FROM entries
                 WHERE session_id = ?1
                 ORDER BY entity_type ASC, entity_id ASC, sequence_number ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([session_id], row_to_entry)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_entry(&self, id: &str) -> Result<Option<Entry>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            "SELECT id, entity_type, entity_id, session_id, sequence_number, entry_type, author_type, author_id, author_model, body, created_at
             FROM entries WHERE id = ?1",
            [id],
            row_to_entry,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

impl<'a> StoreTx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn append_entry(
        &mut self,
        entity_type: EntityType,
        entity_id: &str,
        session_id: Option<&str>,
        entry_type: EntryType,
        author_type: AuthorType,
        author_id: &str,
        author_model: Option<&str>,
        body: &str,
    ) -> Result<Entry, OrbitError> {
        let now = Utc::now();
        let next_sequence = self
            .tx
            .query_row(
                "SELECT COALESCE(MAX(sequence_number), 0) + 1 FROM entries WHERE entity_type = ?1 AND entity_id = ?2",
                params![entity_type.to_string(), entity_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let entry = Entry {
            id: new_id("entry"),
            entity_type,
            entity_id: entity_id.to_string(),
            session_id: session_id.map(ToString::to_string),
            sequence_number: next_sequence,
            entry_type,
            author_type,
            author_id: author_id.to_string(),
            author_model: author_model.map(ToString::to_string),
            body: body.to_string(),
            created_at: now,
        };

        self.tx
            .execute(
                "INSERT INTO entries(id, entity_type, entity_id, session_id, sequence_number, entry_type, author_type, author_id, author_model, body, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.id,
                    entry.entity_type.to_string(),
                    entry.entity_id,
                    entry.session_id,
                    entry.sequence_number,
                    entry.entry_type.to_string(),
                    entry.author_type.to_string(),
                    entry.author_id,
                    entry.author_model,
                    entry.body,
                    entry.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(entry)
    }
}

fn parse_entity_type(raw: &str) -> rusqlite::Result<EntityType> {
    raw.parse::<EntityType>()
        .map_err(|e| enum_parse_error(raw, e))
}

fn parse_entry_type(raw: &str) -> rusqlite::Result<EntryType> {
    raw.parse::<EntryType>()
        .map_err(|e| enum_parse_error(raw, e))
}

fn parse_author_type(raw: &str) -> rusqlite::Result<AuthorType> {
    raw.parse::<AuthorType>()
        .map_err(|e| enum_parse_error(raw, e))
}

fn enum_parse_error(raw: &str, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        raw.len(),
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let entity_type_raw: String = row.get(1)?;
    let entry_type_raw: String = row.get(5)?;
    let author_type_raw: String = row.get(6)?;
    let created_at_raw: String = row.get(10)?;

    Ok(Entry {
        id: row.get(0)?,
        entity_type: parse_entity_type(&entity_type_raw)?,
        entity_id: row.get(2)?,
        session_id: row.get(3)?,
        sequence_number: row.get(4)?,
        entry_type: parse_entry_type(&entry_type_raw)?,
        author_type: parse_author_type(&author_type_raw)?,
        author_id: row.get(7)?,
        author_model: row.get(8)?,
        body: row.get(9)?,
        created_at: parse_timestamp(&created_at_raw)?,
    })
}

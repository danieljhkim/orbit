//! Atomic replacement of checkout-derived ADR and learning indexes.

use orbit_common::types::{Adr, Learning, OrbitError};
use rusqlite::params;

use crate::Store;

impl Store {
    /// Replace either or both knowledge projections in one transaction.
    /// Narrative files are loaded and validated by the caller before entry;
    /// a failure therefore preserves every previously readable selected row.
    pub fn replace_knowledge_indexes(
        &self,
        workspace_id: &str,
        adrs: Option<&[Adr]>,
        learnings: Option<&[Learning]>,
    ) -> Result<(), OrbitError> {
        self.with_transaction(|tx| {
            if let Some(adrs) = adrs {
                tx.tx
                    .execute("DELETE FROM adrs", [])
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                for adr in adrs {
                    insert_adr(&tx.tx, adr)?;
                }
            }
            if let Some(learnings) = learnings {
                tx.tx
                    .execute(
                        "DELETE FROM learnings_index WHERE workspace_id = ?1",
                        [workspace_id],
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                for learning in learnings {
                    insert_learning(&tx.tx, workspace_id, learning)?;
                }
            }
            Ok(())
        })
    }
}

fn insert_adr(conn: &rusqlite::Connection, adr: &Adr) -> Result<(), OrbitError> {
    let related_features = json(&adr.related_features)?;
    let related_tasks = json(&adr.related_tasks)?;
    let tags = json(&adr.tags)?;
    let paths = json(&adr.paths)?;
    let legacy_ids = json(&adr.legacy_ids)?;
    let supersedes = json(&adr.supersedes)?;
    let validation_warnings = json(&adr.validation_warnings)?;
    conn.execute(
        "INSERT INTO adrs (
            id, status, title, owner,
            related_features, related_tasks, tags, paths, legacy_ids, supersedes,
            superseded_by, validation_warnings, legacy_validation,
            created_at, accepted_at, last_updated
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            adr.id,
            adr.status.cli_name(),
            adr.title,
            adr.owner,
            related_features,
            related_tasks,
            tags,
            paths,
            legacy_ids,
            supersedes,
            adr.superseded_by,
            validation_warnings,
            adr.legacy_validation.to_string(),
            adr.created_at.to_rfc3339(),
            adr.accepted_at.map(|timestamp| timestamp.to_rfc3339()),
            adr.last_updated.to_rfc3339(),
        ],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn insert_learning(
    conn: &rusqlite::Connection,
    workspace_id: &str,
    learning: &Learning,
) -> Result<(), OrbitError> {
    conn.execute(
        "INSERT INTO learnings_index (
            workspace_id, id, status, paths, tags, summary, updated_at, priority
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            workspace_id,
            learning.id,
            learning.status.as_str(),
            json(&learning.scope.paths)?,
            json(&learning.scope.tags)?,
            learning.summary,
            learning.updated_at.to_rfc3339(),
            learning.priority.map(i64::from),
        ],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn json(value: &impl serde::Serialize) -> Result<String, OrbitError> {
    serde_json::to_string(value).map_err(|error| OrbitError::Store(error.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{AdrStatus, LearningScope, LearningStatus, LegacyValidation};

    use super::*;

    fn adr(id: &str) -> Adr {
        let now = Utc::now();
        Adr {
            id: id.to_string(),
            title: "ADR".to_string(),
            status: AdrStatus::Proposed,
            owner: "test".to_string(),
            created_at: now,
            accepted_at: None,
            last_updated: now,
            related_features: Vec::new(),
            related_tasks: Vec::new(),
            tags: Vec::new(),
            paths: Vec::new(),
            supersedes: Vec::new(),
            superseded_by: None,
            legacy_ids: Vec::new(),
            validation_warnings: Vec::new(),
            legacy_validation: LegacyValidation::default(),
        }
    }

    fn learning(id: &str) -> Learning {
        let now = Utc::now();
        Learning {
            id: id.to_string(),
            status: LearningStatus::Active,
            scope: LearningScope::default(),
            summary: "Learning".to_string(),
            body: String::new(),
            evidence: Vec::new(),
            supersedes: None,
            superseded_by: None,
            legacy_ids: Vec::new(),
            created_at: now,
            updated_at: now,
            created_by: None,
            priority: None,
        }
    }

    #[test]
    fn combined_replace_rolls_back_both_indexes_when_second_kind_fails() {
        let store = Store::open_in_memory().expect("store");
        store
            .replace_knowledge_indexes(
                "ws-test",
                Some(&[adr("ADR-0001")]),
                Some(&[learning("L-0001")]),
            )
            .expect("seed indexes");
        store
            .with_transaction(|tx| {
                tx.connection()
                    .execute_batch(
                        "CREATE TRIGGER fail_selected_learning
                         BEFORE INSERT ON learnings_index
                         WHEN NEW.id = 'L-FAIL'
                         BEGIN SELECT RAISE(ABORT, 'injected learning index failure'); END;",
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))
            })
            .expect("install failure trigger");

        let error = store
            .replace_knowledge_indexes(
                "ws-test",
                Some(&[adr("ADR-0002")]),
                Some(&[learning("L-FAIL")]),
            )
            .expect_err("combined replacement must fail");
        assert!(
            error
                .to_string()
                .contains("injected learning index failure")
        );

        store
            .with_read_connection(|conn| {
                let adr_id: String = conn
                    .query_row("SELECT id FROM adrs", [], |row| row.get(0))
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                let learning_id: String = conn
                    .query_row(
                        "SELECT id FROM learnings_index WHERE workspace_id = 'ws-test'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                assert_eq!(adr_id, "ADR-0001");
                assert_eq!(learning_id, "L-0001");
                Ok(())
            })
            .expect("prior indexes readable");
    }
}

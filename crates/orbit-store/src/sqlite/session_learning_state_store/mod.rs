use orbit_common::types::{LearningInjectionState, OrbitError};

use crate::{Store, now_string};

impl Store {
    pub fn upsert_session_learning_state(
        &self,
        workspace_id: &str,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        let state_json = serde_json::to_string(state)
            .map_err(|e| OrbitError::Store(format!("serialize session learning state: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute(
            r#"INSERT INTO session_learning_state(
                workspace_id, session_id, learning_injection_state_json, updated_at
            ) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(workspace_id, session_id) DO UPDATE SET
                learning_injection_state_json = excluded.learning_injection_state_json,
                updated_at = excluded.updated_at"#,
            rusqlite::params![workspace_id, session_id, state_json, now_string()],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(())
    }

    pub fn get_session_learning_state(
        &self,
        workspace_id: &str,
        session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT learning_injection_state_json FROM session_learning_state \
                 WHERE workspace_id = ?1 AND session_id = ?2",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let raw = match stmt.query_row(rusqlite::params![workspace_id, session_id], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(raw) => raw,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(err) => return Err(OrbitError::Store(err.to_string())),
        };
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| OrbitError::Store(format!("parse session learning state: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::types::LearningInjectionState;

    use super::*;

    #[test]
    fn upsert_and_get_round_trip() {
        let store = Store::open_in_memory().expect("store");
        let state = LearningInjectionState::seeded(["L-0001".to_string()]);
        store
            .upsert_session_learning_state("ws_a", "session-1", &state)
            .expect("upsert");

        assert_eq!(
            store
                .get_session_learning_state("ws_a", "session-1")
                .expect("get"),
            Some(state)
        );
        assert_eq!(
            store
                .get_session_learning_state("ws_b", "session-1")
                .expect("get other ws"),
            None
        );
    }
}

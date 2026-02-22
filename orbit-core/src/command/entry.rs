use orbit_types::{AuthorType, EntityType, Entry, EntryType, OrbitError, OrbitEvent};

use crate::OrbitRuntime;

#[derive(Debug, Clone)]
pub struct EntryAddParams {
    pub entity_type: EntityType,
    pub entity_id: String,
    pub session_id: Option<String>,
    pub entry_type: EntryType,
    pub author_type: AuthorType,
    pub author_id: String,
    pub author_model: Option<String>,
    pub body: String,
}

impl OrbitRuntime {
    pub fn add_entry(&self, params: EntryAddParams) -> Result<Entry, OrbitError> {
        validate_add_params(&params)?;
        self.validate_entry_entity_exists(params.entity_type, &params.entity_id)?;

        if let Some(ref session_id) = params.session_id
            && self.context.store.get_agent_session(session_id)?.is_none()
        {
            return Err(OrbitError::EntryValidation(format!(
                "session not found: {session_id}"
            )));
        }

        if params.entity_type == EntityType::Session {
            match params.session_id.as_deref() {
                Some(session_id) if session_id == params.entity_id => {}
                Some(session_id) => {
                    return Err(OrbitError::EntryValidation(format!(
                        "session entity requires entity_id == session_id (entity_id={}, session_id={session_id})",
                        params.entity_id
                    )));
                }
                None => {
                    return Err(OrbitError::EntryValidation(
                        "session entity requires session_id".to_string(),
                    ));
                }
            }
        }

        self.with_mutation(|tx| {
            let entry = tx.append_entry(
                params.entity_type,
                &params.entity_id,
                params.session_id.as_deref(),
                params.entry_type,
                params.author_type,
                &params.author_id,
                params.author_model.as_deref(),
                &params.body,
            )?;
            Ok((
                entry.clone(),
                OrbitEvent::EntryCreated {
                    id: entry.id.clone(),
                    entity_type: entry.entity_type.to_string(),
                    entity_id: entry.entity_id.clone(),
                    sequence_number: entry.sequence_number,
                },
            ))
        })
    }

    pub fn list_entries(
        &self,
        entity_type: EntityType,
        entity_id: &str,
    ) -> Result<Vec<Entry>, OrbitError> {
        if entity_id.trim().is_empty() {
            return Err(OrbitError::EntryValidation(
                "entity_id must not be empty".to_string(),
            ));
        }

        self.validate_entry_entity_exists(entity_type, entity_id)?;
        self.context.store.list_entries(entity_type, entity_id)
    }

    pub fn list_entries_by_session(&self, session_id: &str) -> Result<Vec<Entry>, OrbitError> {
        if session_id.trim().is_empty() {
            return Err(OrbitError::EntryValidation(
                "session_id must not be empty".to_string(),
            ));
        }
        if self.context.store.get_agent_session(session_id)?.is_none() {
            return Err(OrbitError::AgentSessionNotFound(session_id.to_string()));
        }
        self.context.store.list_entries_by_session(session_id)
    }

    fn validate_entry_entity_exists(
        &self,
        entity_type: EntityType,
        entity_id: &str,
    ) -> Result<(), OrbitError> {
        let exists = match entity_type {
            EntityType::Task => self.context.store.get_task(entity_id)?.is_some(),
            EntityType::Job => self.context.store.get_job(entity_id)?.is_some(),
            EntityType::Watch => self.context.store.get_watch(entity_id)?.is_some(),
            EntityType::Session => self.context.store.get_agent_session(entity_id)?.is_some(),
            EntityType::Workflow => {
                return Err(OrbitError::EntryValidation(
                    "unsupported entity type in v1: workflow".to_string(),
                ));
            }
        };

        if exists {
            Ok(())
        } else {
            Err(OrbitError::EntryValidation(format!(
                "{entity_type} entity not found: {entity_id}"
            )))
        }
    }
}

fn validate_add_params(params: &EntryAddParams) -> Result<(), OrbitError> {
    if params.entity_id.trim().is_empty() {
        return Err(OrbitError::EntryValidation(
            "entity_id must not be empty".to_string(),
        ));
    }

    if let Some(ref session_id) = params.session_id
        && session_id.trim().is_empty()
    {
        return Err(OrbitError::EntryValidation(
            "session_id must not be empty when provided".to_string(),
        ));
    }

    if params.author_id.trim().is_empty() {
        return Err(OrbitError::EntryValidation(
            "author_id must not be empty".to_string(),
        ));
    }

    if params.body.trim().is_empty() {
        return Err(OrbitError::EntryValidation(
            "body must not be empty".to_string(),
        ));
    }

    if matches!(params.author_type, AuthorType::Agent) {
        let model = params
            .author_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if model.is_none() {
            return Err(OrbitError::EntryValidation(
                "author_model is required when author_type=agent".to_string(),
            ));
        }
    }

    if let Some(ref model) = params.author_model
        && model.trim().is_empty()
    {
        return Err(OrbitError::EntryValidation(
            "author_model must not be empty when provided".to_string(),
        ));
    }

    Ok(())
}

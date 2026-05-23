use super::*;

impl TaskV2Store {
    pub(super) fn task_matches_query(
        &self,
        task: &Task,
        lowered: &str,
    ) -> Result<bool, OrbitError> {
        if task_in_memory_fields_match_query(task, lowered) {
            return Ok(true);
        }

        if self.task_sidecars_match_query(&task.id, lowered)? {
            return Ok(true);
        }

        // Phase 5 bridge: artifact search reads text artifact files on demand until
        // generated full-text indexes carry artifact paths, content, and snippets.
        self.task_artifacts_match_query(&task.id, lowered)
    }

    fn task_sidecars_match_query(&self, id: &str, lowered: &str) -> Result<bool, OrbitError> {
        let Some(comments) = self.get_task_comments(id)? else {
            return Ok(false);
        };
        if comments
            .iter()
            .any(|comment| comment.message.to_lowercase().contains(lowered))
        {
            return Ok(true);
        }
        let Some(review_threads) = self.get_task_review_threads(id)? else {
            return Ok(false);
        };
        Ok(review_threads.iter().any(|thread| {
            thread.messages.iter().any(|message| {
                message.body.to_lowercase().contains(lowered)
                    || message.by.to_lowercase().contains(lowered)
            }) || thread
                .path
                .as_deref()
                .is_some_and(|path| path.to_lowercase().contains(lowered))
        }))
    }

    fn task_artifacts_match_query(&self, id: &str, lowered: &str) -> Result<bool, OrbitError> {
        let Some(artifacts) = self.get_task_artifacts(id)? else {
            // A task may be deleted after the indexed/listed candidate set is built.
            return Ok(false);
        };
        Ok(artifacts.iter().any(|artifact| {
            artifact.path.to_lowercase().contains(lowered)
                || (is_text_artifact_media_type(&artifact.media_type)
                    && artifact
                        .text_content()
                        .is_some_and(|content| content.to_lowercase().contains(lowered)))
        }))
    }
}

fn task_in_memory_fields_match_query(task: &Task, lowered: &str) -> bool {
    task.title.to_lowercase().contains(lowered)
        || task.description.to_lowercase().contains(lowered)
        || task.plan.to_lowercase().contains(lowered)
        || task.execution_summary.to_lowercase().contains(lowered)
        || task
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.to_lowercase().contains(lowered))
        || task.external_refs.iter().any(|external_ref| {
            external_ref.system.to_lowercase().contains(lowered)
                || external_ref.id.to_lowercase().contains(lowered)
        })
}

fn is_text_artifact_media_type(media_type: &str) -> bool {
    let base = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    base.starts_with("text/")
        || matches!(
            base.as_str(),
            "application/json"
                | "application/javascript"
                | "application/toml"
                | "application/x-toml"
                | "application/x-yaml"
                | "application/xml"
                | "application/yaml"
        )
        || base.ends_with("+json")
        || base.ends_with("+xml")
}

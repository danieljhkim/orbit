use super::*;

pub(super) fn review_thread_from_v2(thread: TaskReviewThreadV2) -> ReviewThread {
    let bodies = review_message_bodies(&thread.body);
    let mut fallback_body = Some(thread.body);
    ReviewThread {
        thread_id: thread.metadata.thread_id,
        path: thread.metadata.path,
        line: thread.metadata.line,
        status: thread.metadata.status,
        messages: thread
            .metadata
            .messages
            .into_iter()
            .map(|message| ReviewMessage {
                body: bodies
                    .get(&message.message_id)
                    .cloned()
                    .unwrap_or_else(|| fallback_body.take().unwrap_or_default()),
                message_id: message.message_id,
                at: message.at,
                by: message.by,
                github_comment_id: message.github_comment_id,
            })
            .collect(),
        github_thread_id: thread.metadata.github_thread_id,
    }
}

pub(super) fn review_thread_to_v2(thread: ReviewThread) -> Result<TaskReviewThreadV2, OrbitError> {
    let now = Utc::now();
    let created_at = thread
        .messages
        .first()
        .map(|message| message.at)
        .unwrap_or(now);
    let updated_at = thread
        .messages
        .last()
        .map(|message| message.at)
        .unwrap_or(now);
    let body = render_review_thread_body(&thread.messages);
    let metadata = ReviewThreadMetadataV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        thread_id: thread.thread_id,
        status: thread.status,
        path: thread
            .path
            .as_deref()
            .map(normalize_v2_artifact_path)
            .transpose()?,
        line: thread.line,
        github_thread_id: thread.github_thread_id,
        messages: thread
            .messages
            .into_iter()
            .map(|message| ReviewThreadMessageMetadataV2 {
                message_id: message.message_id,
                at: message.at,
                by: message.by,
                github_comment_id: message.github_comment_id,
            })
            .collect(),
        created_at,
        updated_at,
    };
    metadata.validate()?;
    Ok(TaskReviewThreadV2 { metadata, body })
}

pub(super) fn merge_review_threads_v2(
    existing: &mut Vec<ReviewThread>,
    incoming: Vec<ReviewThread>,
) {
    for thread in incoming {
        if let Some(existing_thread) = existing
            .iter_mut()
            .find(|candidate| candidate.thread_id == thread.thread_id)
        {
            existing_thread.messages.extend(thread.messages);
            existing_thread.status = thread.status;
            if thread.github_thread_id.is_some() {
                existing_thread.github_thread_id = thread.github_thread_id;
            }
        } else {
            existing.push(thread);
        }
    }
}

fn render_review_thread_body(messages: &[ReviewMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str("<!-- orbit-review-message:");
        out.push_str(&message.message_id);
        out.push_str(" -->\n");
        for line in message.body.trim_end().lines() {
            if review_message_anchor_id(line).is_some() {
                out.push('\\');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\n\n");
    }
    out
}

fn review_message_bodies(body: &str) -> BTreeMap<String, String> {
    let mut bodies = BTreeMap::new();
    let mut current_id: Option<String> = None;
    let mut current_body = String::new();
    for line in body.lines() {
        if let Some(message_id) = review_message_anchor_id(line) {
            if let Some(id) = current_id.replace(message_id) {
                bodies.insert(id, current_body.trim_end().to_string());
                current_body.clear();
            }
            continue;
        }
        if current_id.is_some() {
            if let Some(escaped) = line.strip_prefix('\\')
                && review_message_anchor_id(escaped).is_some()
            {
                current_body.push_str(escaped);
            } else {
                current_body.push_str(line);
            }
            current_body.push('\n');
        }
    }
    if let Some(id) = current_id {
        bodies.insert(id, current_body.trim_end().to_string());
    }
    bodies
}

fn review_message_anchor_id(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix("<!-- orbit-review-message:")
        .and_then(|value| value.strip_suffix(" -->"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

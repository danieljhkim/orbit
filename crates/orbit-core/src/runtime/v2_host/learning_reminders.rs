use std::collections::BTreeMap;

use orbit_common::types::{
    LearningInjectionCaps, LearningReminder, OrbitError, Task, normalize_learning_tags,
};
use orbit_common::utility::selector::anchor_path;
use orbit_engine::DispatchError;
use orbit_store::{LearningSearchParams, LearningSearchResult};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::task::{canonicalize_context_files_for_read, context_workspace_root};
use crate::runtime::run_input::singular_task_id_from_input;

pub(crate) fn learning_reminders_for_task(
    runtime: &OrbitRuntime,
    input: &Value,
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, DispatchError> {
    let Some(task_id) = singular_task_id_from_input(input) else {
        return Ok(Vec::new());
    };
    let task = runtime.get_task(task_id).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load task `{task_id}` for learning reminders: {err}"
        ))
    })?;
    learning_reminders_for_task_snapshot(runtime, &task, input, caps).map_err(|err| {
        DispatchError::CliInvocationFailed(format!("search learnings for task `{task_id}`: {err}"))
    })
}

fn learning_reminders_for_task_snapshot(
    runtime: &OrbitRuntime,
    task: &Task,
    input: &Value,
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, OrbitError> {
    let mut batches = Vec::new();
    for path in task_context_paths(runtime, task, input) {
        batches.push(runtime.search_learnings(LearningSearchParams {
            path: Some(path),
            tag: None,
            query: None,
            limit: None,
        })?);
    }
    for tag in normalize_learning_tags(task.tags.clone()) {
        batches.push(runtime.search_learnings(LearningSearchParams {
            path: None,
            tag: Some(tag),
            query: None,
            limit: None,
        })?);
    }

    let mut reminders = Vec::new();
    for result in merge_ranked_results(batches, caps.per_call) {
        let id = result.learning.id;
        // Skip index-only ghosts: the SQLite envelope can outlive its YAML
        // body after a partial rollback or manual removal. Reminders are a
        // read-side surface, so a stale index row must not inject a summary
        // for a record that no longer resolves on disk. Previously this was
        // enforced as a side effect of the (now-removed) comment-hydration
        // step erroring out; this explicit `get_learning` check preserves
        // the same guarantee.
        if let Err(err) = runtime.get_learning(&id) {
            orbit_common::tracing::warn!(
                target: "orbit.core.learning_reminders",
                learning_id = id.as_str(),
                error = %err,
                "skipping learning reminder because the YAML body is missing",
            );
            continue;
        }
        reminders.push(LearningReminder {
            id,
            summary: result.learning.summary,
            tags: result.learning.scope.tags,
        });
    }
    Ok(reminders)
}

fn task_context_paths(runtime: &OrbitRuntime, task: &Task, input: &Value) -> Vec<String> {
    let workspace_path = input.get("workspace_path").and_then(Value::as_str);
    let prune_root = context_workspace_root(&runtime.paths().repo_root, workspace_path);
    let canonical_context_files =
        canonicalize_context_files_for_read(&task.context_files, &prune_root);
    let mut paths = Vec::new();
    for selector in canonical_context_files {
        let Ok(path) = anchor_path(&selector) else {
            continue;
        };
        let path = path.to_string_lossy().replace('\\', "/");
        if !path.is_empty() && !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}

fn merge_ranked_results(
    batches: Vec<Vec<LearningSearchResult>>,
    limit: usize,
) -> Vec<LearningSearchResult> {
    let mut by_id: BTreeMap<String, LearningSearchResult> = BTreeMap::new();
    for result in batches.into_iter().flatten() {
        by_id
            .entry(result.learning.id.clone())
            .and_modify(|existing| merge_matched_by(existing, &result))
            .or_insert(result);
    }
    let mut merged: Vec<_> = by_id.into_values().collect();
    merged.sort_by(|a, b| {
        priority_rank(b.learning.priority)
            .cmp(&priority_rank(a.learning.priority))
            .then_with(|| b.learning.updated_at.cmp(&a.learning.updated_at))
            .then_with(|| a.learning.id.cmp(&b.learning.id))
    });
    merged.truncate(limit);
    merged
}

fn merge_matched_by(existing: &mut LearningSearchResult, incoming: &LearningSearchResult) {
    for axis in &incoming.matched_by {
        if !existing.matched_by.iter().any(|seen| seen == axis) {
            existing.matched_by.push(axis.clone());
        }
    }
}

fn priority_rank(priority: Option<u8>) -> i16 {
    priority.map(i16::from).unwrap_or(-1)
}

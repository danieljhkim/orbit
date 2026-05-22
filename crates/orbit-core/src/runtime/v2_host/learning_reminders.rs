use std::collections::BTreeMap;

use orbit_common::types::{
    LearningInjectionCaps, LearningReminder, OrbitError, Task, normalize_learning_tags,
    read_comment_render_cap_env,
};
use orbit_common::utility::selector::anchor_path;
use orbit_engine::DispatchError;
use orbit_store::{LearningSearchParams, LearningSearchResult};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::task::{canonicalize_context_files_for_read, context_workspace_root};
use crate::runtime::run_input::singular_task_id_from_input;

pub(super) fn learning_reminders_for_task(
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

    let comment_cap = read_comment_render_cap_env();
    let mut reminders = Vec::new();
    for result in merge_ranked_results(batches, caps.per_call) {
        let id = result.learning.id;
        let comments = match runtime.list_learning_comments(&id, false) {
            Ok(comments) => comments.into_iter().take(comment_cap).collect(),
            Err(err) => {
                orbit_common::tracing::warn!(
                    target: "orbit.core.learning_reminders",
                    learning_id = id.as_str(),
                    error = %err,
                    "skipping learning reminder after comment hydration failed",
                );
                continue;
            }
        };
        reminders.push(LearningReminder {
            id,
            summary: result.learning.summary,
            comments,
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use orbit_common::types::{LearningScope, Task};
    use orbit_engine::V2RuntimeHost;
    use orbit_store::LearningCreateParams;
    use serde_json::json;

    use super::*;
    use crate::OrbitRuntime;
    use crate::command::task::TaskAddParams;

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        value: Option<String>,
    }

    fn set_comment_cap_env(value: Option<&str>) -> EnvGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("ORBIT_LEARNING_COMMENT_RENDER_CAP").ok();
        unsafe {
            match value {
                Some(value) => std::env::set_var("ORBIT_LEARNING_COMMENT_RENDER_CAP", value),
                None => std::env::remove_var("ORBIT_LEARNING_COMMENT_RENDER_CAP"),
            }
        }
        EnvGuard {
            _lock: lock,
            value: previous,
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.value {
                    Some(value) => std::env::set_var("ORBIT_LEARNING_COMMENT_RENDER_CAP", value),
                    None => std::env::remove_var("ORBIT_LEARNING_COMMENT_RENDER_CAP"),
                }
            }
        }
    }

    fn create_learning(
        runtime: &OrbitRuntime,
        summary: &str,
        paths: &[&str],
        tags: &[&str],
        priority: Option<u8>,
    ) -> orbit_common::types::Learning {
        runtime
            .create_learning(LearningCreateParams {
                summary: summary.to_string(),
                scope: LearningScope {
                    paths: paths.iter().map(|value| (*value).to_string()).collect(),
                    tags: tags.iter().map(|value| (*value).to_string()).collect(),
                    ..Default::default()
                },
                body: "body must not be injected".to_string(),
                evidence: Vec::new(),
                created_by: Some("gpt-5.5".to_string()),
                priority,
            })
            .expect("create learning")
    }

    fn task_with_context(
        runtime: &OrbitRuntime,
        context_files: Vec<String>,
        tags: Vec<String>,
    ) -> Task {
        std::fs::create_dir_all(runtime.paths().repo_root.join("crates/orbit-engine/src"))
            .expect("create context dir");
        runtime
            .add_task(TaskAddParams {
                title: "Learning reminder task".to_string(),
                description: "Task description.".to_string(),
                acceptance_criteria: vec!["works".to_string()],
                plan: "plan".to_string(),
                context_files,
                tags,
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task")
    }

    #[test]
    fn reminders_match_task_context_paths_and_tags_without_body() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        create_learning(
            &runtime,
            "Remember the engine path.",
            &["crates/orbit-engine/**"],
            &[],
            None,
        );
        create_learning(&runtime, "Remember the tag.", &[], &["workflow"], None);
        let task = task_with_context(
            &runtime,
            vec!["dir:crates/orbit-engine/src".to_string()],
            vec!["workflow".to_string()],
        );

        let reminders = runtime
            .learning_reminders_for_task(
                &json!({"task_id": task.id}),
                LearningInjectionCaps::default(),
            )
            .expect("learning reminders");

        assert_eq!(reminders.len(), 2);
        assert!(
            reminders
                .iter()
                .any(|reminder| reminder.summary == "Remember the engine path.")
        );
        assert!(
            reminders
                .iter()
                .any(|reminder| reminder.summary == "Remember the tag.")
        );
        assert!(
            !serde_json::to_string(&reminders)
                .expect("json")
                .contains("body")
        );
    }

    #[test]
    fn reminders_apply_default_per_call_cap_after_merge() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        for idx in 0..7 {
            create_learning(
                &runtime,
                &format!("Learning {idx}"),
                &["crates/orbit-engine/**"],
                &[],
                Some(idx),
            );
        }
        let task = task_with_context(
            &runtime,
            vec!["dir:crates/orbit-engine/src".to_string()],
            Vec::new(),
        );

        let reminders = runtime
            .learning_reminders_for_task(
                &json!({"task_id": task.id}),
                LearningInjectionCaps::default(),
            )
            .expect("learning reminders");

        assert_eq!(reminders.len(), 5);
        assert_eq!(reminders[0].summary, "Learning 6");
    }

    #[test]
    fn reminders_skip_indexed_learning_when_yaml_is_missing() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let missing = create_learning(
            &runtime,
            "Missing YAML reminder.",
            &["crates/orbit-engine/**"],
            &[],
            Some(2),
        );
        create_learning(
            &runtime,
            "Available reminder.",
            &["crates/orbit-engine/**"],
            &[],
            Some(1),
        );
        let task = task_with_context(
            &runtime,
            vec!["dir:crates/orbit-engine/src".to_string()],
            Vec::new(),
        );
        std::fs::remove_file(
            runtime
                .paths()
                .learnings_dir
                .join(&missing.id)
                .join("learning.yaml"),
        )
        .expect("remove learning yaml");

        let search_results = runtime
            .search_learnings(LearningSearchParams {
                path: Some("crates/orbit-engine/src".to_string()),
                tag: None,
                query: None,
                limit: None,
            })
            .expect("search indexed learnings");
        assert!(
            search_results
                .iter()
                .any(|result| result.learning.id == missing.id)
        );

        let reminders = runtime
            .learning_reminders_for_task(
                &json!({"task_id": task.id}),
                LearningInjectionCaps::default(),
            )
            .expect("learning reminders");

        assert_eq!(reminders.len(), 1);
        assert_eq!(reminders[0].summary, "Available reminder.");
        assert!(!reminders.iter().any(|reminder| reminder.id == missing.id));
    }

    #[test]
    fn reminders_attach_active_comments_oldest_first_with_render_cap() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let learning = create_learning(
            &runtime,
            "Remember the engine path.",
            &["crates/orbit-engine/**"],
            &[],
            None,
        );
        let mut comment_ids = Vec::new();
        for idx in 0..5 {
            let comment = runtime
                .add_learning_comment(
                    learning.id.clone(),
                    format!("comment {idx}"),
                    "codex".to_string(),
                )
                .expect("add comment");
            comment_ids.push(comment.id);
        }
        runtime
            .delete_learning_comment(comment_ids[1].clone(), Some("codex".to_string()))
            .expect("delete one");
        runtime
            .delete_learning_comment(comment_ids[3].clone(), Some("codex".to_string()))
            .expect("delete two");
        let task = task_with_context(
            &runtime,
            vec!["dir:crates/orbit-engine/src".to_string()],
            Vec::new(),
        );

        {
            let _env = set_comment_cap_env(None);
            let reminders = runtime
                .learning_reminders_for_task(
                    &json!({"task_id": task.id.clone()}),
                    LearningInjectionCaps::default(),
                )
                .expect("learning reminders");
            let block = orbit_common::types::render_reminder_block(&reminders);
            assert_eq!(
                reminders[0]
                    .comments
                    .iter()
                    .map(|comment| comment.body.as_str())
                    .collect::<Vec<_>>(),
                vec!["comment 0", "comment 2", "comment 4"]
            );
            assert!(block.contains("comment 0"));
            assert!(block.contains("comment 2"));
            assert!(block.contains("comment 4"));
            assert!(!block.contains("comment 1"));
            assert!(!block.contains("comment 3"));
        }

        {
            let _env = set_comment_cap_env(Some("1"));
            let reminders = runtime
                .learning_reminders_for_task(
                    &json!({"task_id": task.id.clone()}),
                    LearningInjectionCaps::default(),
                )
                .expect("learning reminders");
            let block = orbit_common::types::render_reminder_block(&reminders);
            assert_eq!(
                reminders[0]
                    .comments
                    .iter()
                    .map(|comment| comment.body.as_str())
                    .collect::<Vec<_>>(),
                vec!["comment 0"]
            );
            assert!(block.contains("comment 0"));
            assert!(!block.contains("comment 2"));
        }
    }
}

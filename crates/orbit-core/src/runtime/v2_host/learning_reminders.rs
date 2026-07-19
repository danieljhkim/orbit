use std::collections::BTreeMap;

use orbit_common::types::{
    LearningReminder, OrbitError, Task, UpfrontLearningReminderBatch, normalize_learning_tags,
};
use orbit_engine::DispatchError;
use orbit_store::{LearningSearchParams, LearningSearchResult};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::run_input::singular_task_id_from_input;

pub(super) fn learning_reminders_for_task(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<UpfrontLearningReminderBatch, DispatchError> {
    let Some(task_id) = singular_task_id_from_input(input) else {
        return Ok(UpfrontLearningReminderBatch::empty());
    };
    let task = runtime.get_task(task_id).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load task `{task_id}` for learning reminders: {err}"
        ))
    })?;
    learning_reminders_for_task_snapshot(runtime, &task, input).map_err(|err| {
        DispatchError::CliInvocationFailed(format!("search learnings for task `{task_id}`: {err}"))
    })
}

fn learning_reminders_for_task_snapshot(
    runtime: &OrbitRuntime,
    task: &Task,
    _input: &Value,
) -> Result<UpfrontLearningReminderBatch, OrbitError> {
    let Some(config) = orbit_store::sqlite::task_registry::read_workspace_config_optional(
        &runtime.paths().orbit_dir,
    )?
    else {
        return Ok(UpfrontLearningReminderBatch::empty());
    };
    let cap = config.learnings.upfront_injection_cap;
    if cap == 0 || config.learnings.tag_vocabulary.is_empty() {
        return Ok(UpfrontLearningReminderBatch {
            reminders: Vec::new(),
            cap,
        });
    }
    let vocabulary = config
        .learnings
        .tag_vocabulary
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let tags = normalize_learning_tags(task.tags.clone())
        .into_iter()
        .filter(|tag| vocabulary.contains(tag))
        .collect::<Vec<_>>();
    if tags.is_empty() {
        return Ok(UpfrontLearningReminderBatch {
            reminders: Vec::new(),
            cap,
        });
    }

    let mut batches = Vec::new();
    for tag in tags {
        batches.push(runtime.search_learnings(LearningSearchParams {
            path: None,
            tag: Some(tag),
            query: None,
            limit: None,
        })?);
    }

    let mut reminders = Vec::new();
    for result in merge_ranked_results(batches, cap) {
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
        });
    }
    Ok(UpfrontLearningReminderBatch { reminders, cap })
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
        b.matched_by
            .len()
            .cmp(&a.matched_by.len())
            .then_with(|| {
                priority_rank(b.learning.priority).cmp(&priority_rank(a.learning.priority))
            })
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
    use orbit_common::types::{LearningScope, Task};
    use orbit_engine::V2RuntimeHost;
    use orbit_store::LearningCreateParams;
    use orbit_store::sqlite::task_registry::{read_workspace_config, write_workspace_config};
    use serde_json::json;

    use super::*;
    use crate::OrbitRuntime;
    use crate::command::task::TaskAddParams;

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
                created_by: Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
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
    fn reminders_match_task_tags_only_without_body() {
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
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("learning reminders");

        assert_eq!(reminders.cap, 5);
        assert_eq!(reminders.reminders.len(), 1);
        assert!(
            reminders
                .reminders
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
    fn reminders_are_empty_for_no_tag_task() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        create_learning(&runtime, "Tagged.", &[], &["workflow"], None);
        let task = task_with_context(&runtime, Vec::new(), Vec::new());

        let batch = runtime
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("learning reminders");

        assert!(batch.reminders.is_empty());
    }

    #[test]
    fn reminders_fail_open_for_empty_vocabulary_and_missing_config() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        create_learning(&runtime, "Tagged.", &[], &["workflow"], None);
        let task = task_with_context(&runtime, Vec::new(), vec!["workflow".to_string()]);
        let config_path = runtime.paths().orbit_dir.join("config.yaml");
        let mut config = read_workspace_config(&runtime.paths().orbit_dir).expect("config");
        config.learnings.tag_vocabulary.clear();
        write_workspace_config(&runtime.paths().orbit_dir, &config).expect("empty vocabulary");

        let empty = runtime
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("empty vocabulary");
        assert!(empty.reminders.is_empty());

        std::fs::remove_file(config_path).expect("remove config");
        let missing = runtime
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("missing config must fail open");
        assert!(missing.reminders.is_empty());
    }

    #[test]
    fn reminders_rank_match_strength_before_priority_and_enforce_configured_cap() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        create_learning(
            &runtime,
            "Two matching tags, low priority",
            &[],
            &["workflow", "testing"],
            Some(0),
        );
        create_learning(
            &runtime,
            "One matching tag, high priority",
            &[],
            &["workflow"],
            Some(9),
        );
        for idx in 0..4 {
            create_learning(
                &runtime,
                &format!("Learning {idx}"),
                &[],
                &["workflow"],
                Some(idx),
            );
        }
        let task = task_with_context(
            &runtime,
            Vec::new(),
            vec!["workflow".to_string(), "testing".to_string()],
        );
        let mut config = read_workspace_config(&runtime.paths().orbit_dir).expect("config");
        config.learnings.upfront_injection_cap = 2;
        write_workspace_config(&runtime.paths().orbit_dir, &config).expect("write cap");

        let batch = runtime
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("learning reminders");

        assert_eq!(batch.cap, 2);
        assert_eq!(batch.reminders.len(), 2);
        assert_eq!(
            batch.reminders[0].summary,
            "Two matching tags, low priority"
        );
        assert_eq!(
            batch.reminders[1].summary,
            "One matching tag, high priority"
        );
    }

    #[test]
    fn upfront_injection_records_global_learning_injected_audit_event() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let admitted = vec![LearningReminder {
            id: "L-0001".to_string(),
            summary: "Audit this injection.".to_string(),
        }];

        runtime
            .record_learning_injected(
                "job_run_start",
                Some("ORB-00001"),
                "jrun-audit",
                "session-audit",
                &admitted,
            )
            .expect("record audit");

        let events = runtime
            .list_audit_events_with_kind(
                None,
                Some("agent_invoke".to_string()),
                Some("learning_injected".to_string()),
                None,
                None,
                10,
            )
            .expect("list audit");
        let event = events.first().expect("learning audit event");
        assert_eq!(event.command, "job");
        assert_eq!(event.subcommand.as_deref(), Some("run-start"));
        assert_eq!(event.task_id.as_deref(), Some("ORB-00001"));
        assert_eq!(event.job_run_id.as_deref(), Some("jrun-audit"));
        assert_eq!(event.session_id.as_deref(), Some("session-audit"));
        let arguments: Value =
            serde_json::from_str(event.arguments_json.as_deref().expect("audit arguments"))
                .expect("parse audit arguments");
        assert_eq!(arguments["surface"], "job_run_start");
        assert_eq!(arguments["learning_ids"], json!(["L-0001"]));
    }

    #[test]
    fn reminders_skip_indexed_learning_when_yaml_is_missing() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let missing = create_learning(
            &runtime,
            "Missing YAML reminder.",
            &[],
            &["workflow"],
            Some(2),
        );
        create_learning(&runtime, "Available reminder.", &[], &["workflow"], Some(1));
        let task = task_with_context(&runtime, Vec::new(), vec!["workflow".to_string()]);
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
                path: None,
                tag: Some("workflow".to_string()),
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
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("learning reminders");

        assert_eq!(reminders.reminders.len(), 1);
        assert_eq!(reminders.reminders[0].summary, "Available reminder.");
        assert!(
            !reminders
                .reminders
                .iter()
                .any(|reminder| reminder.id == missing.id)
        );
    }

    /// ORB-10113: two workspaces bound to the same host-global database must
    /// not cross-pollinate reminders. Workspace A's task never receives
    /// workspace B's learning summary, while a genuine same-workspace index
    /// row whose YAML body is missing is still skipped by the defensive check.
    #[test]
    fn reminders_never_cross_workspaces_but_skip_same_workspace_ghosts() {
        let root = tempfile::tempdir().expect("tempdir");
        let global_root = root.path().join("global");
        std::fs::create_dir_all(&global_root).expect("global root");
        let ws_a = root.path().join("repo-a").join(".orbit");
        let ws_b = root.path().join("repo-b").join(".orbit");
        std::fs::create_dir_all(&ws_a).expect("ws a");
        std::fs::create_dir_all(&ws_b).expect("ws b");

        // Both runtimes share `global_root/orbit.db`, so they share the single
        // `learnings_index` table but have distinct registered workspace ids.
        let runtime_a = OrbitRuntime::from_roots(&global_root, &ws_a).expect("runtime a");
        let runtime_b = OrbitRuntime::from_roots(&global_root, &ws_b).expect("runtime b");

        // Workspace B owns a learning on a shared path glob. Its canonical id
        // is `L-0001`, colliding with workspace A's first record — the exact
        // duplicate-id shape that let a foreign summary leak in before scoping.
        create_learning(&runtime_b, "workspace B leak", &[], &["workflow"], Some(9));

        // Workspace A: a genuine same-workspace ghost (index row whose YAML is
        // removed) plus a live record. The ghost must be skipped, the live one
        // kept, and B's row must never appear.
        let ghost = create_learning(&runtime_a, "workspace A ghost", &[], &["workflow"], Some(5));
        create_learning(&runtime_a, "workspace A live", &[], &["workflow"], Some(1));
        std::fs::remove_file(
            runtime_a
                .paths()
                .learnings_dir
                .join(&ghost.id)
                .join("learning.yaml"),
        )
        .expect("remove ghost yaml");

        let task = task_with_context(&runtime_a, Vec::new(), vec!["workflow".to_string()]);
        let reminders = runtime_a
            .learning_reminders_for_task(&json!({"task_id": task.id}))
            .expect("learning reminders");

        assert_eq!(
            reminders.reminders.len(),
            1,
            "only workspace A's live learning should remind: {reminders:?}"
        );
        assert_eq!(reminders.reminders[0].summary, "workspace A live");
        assert!(
            !reminders
                .reminders
                .iter()
                .any(|r| r.summary == "workspace B leak"),
            "workspace B's learning must not cross into workspace A",
        );
        assert!(
            !reminders.reminders.iter().any(|r| r.id == ghost.id),
            "the same-workspace ghost (missing YAML) must be skipped",
        );
    }
}

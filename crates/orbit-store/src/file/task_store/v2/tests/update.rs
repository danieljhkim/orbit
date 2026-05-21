use super::*;

#[test]
fn document_update_rewrites_v2_documents_and_envelope() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    store
        .create_task(create_params("Original", TaskStatus::Backlog))
        .expect("create task");

    store
        .update_task_document(
            "ORB-00000",
            &TaskDocumentUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                title: Some("Renamed".to_string()),
                description: Some("Updated description".to_string()),
                acceptance_criteria: Some(vec!["Updated criterion".to_string()]),
                tags: Some(vec!["v2".to_string(), "store".to_string()]),
                plan: Some("1. Updated plan".to_string()),
                execution_summary: Some("Updated summary".to_string()),
                priority: Some(TaskPriority::Low),
                ..Default::default()
            },
        )
        .expect("update document");

    let task = store
        .get_task("ORB-00000")
        .expect("get task")
        .expect("task exists");
    assert_eq!(task.title, "Renamed");
    assert_eq!(task.description, "Updated description");
    assert_eq!(task.acceptance_criteria, vec!["Updated criterion"]);
    assert_eq!(task.tags, vec!["v2", "store"]);
    assert_eq!(task.plan, "1. Updated plan");
    assert_eq!(task.execution_summary, "Updated summary");
    assert_eq!(task.priority, TaskPriority::Low);
    assert!(
        store
            .get_task_history("ORB-00000")
            .expect("get history")
            .expect("task exists")
            .iter()
            .any(|entry| entry.event == "renamed")
    );
    assert_eq!(
        store
            .list_tasks_by_tags(&["task-artifacts".to_string()])
            .expect("old tag should leave generated index")
            .len(),
        0
    );
    assert_eq!(
        store
            .list_tasks_filtered(None, Some(TaskPriority::Low), None, None, None, None)
            .expect("priority filter should use updated generated index")
            .len(),
        1
    );
}

#[test]
fn document_update_sets_and_clears_source_task_id() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let source = store
        .create_task(create_params("Source", TaskStatus::Done))
        .expect("create source");
    store
        .create_task(create_params("Bug", TaskStatus::Backlog))
        .expect("create bug");

    store
        .update_task_document(
            "ORB-00001",
            &TaskDocumentUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                source_task_id: Some(Some(source.id.clone())),
                ..Default::default()
            },
        )
        .expect("set source task");

    let task = store
        .get_task("ORB-00001")
        .expect("get task")
        .expect("task exists");
    assert_eq!(task.source_task_id(), Some(source.id.as_str()));
    let envelope = store
        .bundle_store
        .read_bundle("ORB-00001")
        .expect("read bundle")
        .envelope;
    assert!(envelope.relations.iter().any(|relation| {
        relation.relation_type == TaskRelationType::RegressionFrom && relation.target == source.id
    }));

    store
        .update_task_document(
            "ORB-00001",
            &TaskDocumentUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                source_task_id: Some(None),
                ..Default::default()
            },
        )
        .expect("clear source task");

    let task = store
        .get_task("ORB-00001")
        .expect("get task")
        .expect("task exists");
    assert_eq!(task.source_task_id(), None);
    let envelope = store
        .bundle_store
        .read_bundle("ORB-00001")
        .expect("read bundle")
        .envelope;
    assert!(
        envelope
            .relations
            .iter()
            .all(|relation| relation.relation_type != TaskRelationType::RegressionFrom)
    );
}

#[test]
fn history_update_appends_comments_and_status_events() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    store
        .create_task(create_params("History", TaskStatus::Backlog))
        .expect("create task");
    let at = Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap();

    store
        .update_task_history(
            "ORB-00000",
            &TaskHistoryUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                status: Some(TaskStatus::InProgress),
                status_note: Some("Starting".to_string()),
                append_history: vec![TaskHistoryEntry {
                    at,
                    by: "codex:gpt-5.5".to_string(),
                    event: "context_pruned".to_string(),
                    note: Some("Dropped missing file".to_string()),
                    from_status: None,
                    to_status: None,
                }],
                append_comments: vec![TaskComment {
                    at,
                    by: "codex:gpt-5.5".to_string(),
                    message: "Working on it".to_string(),
                }],
                ..Default::default()
            },
        )
        .expect("update history");

    let task = store
        .get_task("ORB-00000")
        .expect("get task")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::InProgress);
    assert_eq!(
        store
            .list_tasks_filtered(Some(TaskStatus::InProgress), None, None, None, None, None,)
            .expect("status filter should use updated generated index")
            .len(),
        1
    );
    let comments = store
        .get_task_comments("ORB-00000")
        .expect("get comments")
        .expect("task exists");
    assert!(
        comments
            .iter()
            .any(|comment| comment.message == "Working on it")
    );
    let history = store
        .get_task_history("ORB-00000")
        .expect("get history")
        .expect("task exists");
    let status_event = history
        .iter()
        .find(|event| event.event == "status_changed")
        .expect("status event");
    assert_eq!(status_event.from_status, Some(TaskStatus::Backlog));
    assert_eq!(status_event.to_status, Some(TaskStatus::InProgress));
    assert_eq!(status_event.note.as_deref(), Some("Starting"));
}

#[test]
fn review_update_merges_replies_resolves_and_replaces_threads() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    store
        .create_task(create_params("Reviews", TaskStatus::Backlog))
        .expect("create task");
    let at = Utc.with_ymd_and_hms(2026, 5, 11, 14, 0, 0).unwrap();

    store
        .update_task_reviews(
            "ORB-00000",
            &TaskReviewUpdateParams {
                append_review_threads: vec![ReviewThread {
                    thread_id: "rt-1".to_string(),
                    path: Some("./src/lib.rs".to_string()),
                    line: Some(12),
                    status: ReviewThreadStatus::Open,
                    messages: vec![ReviewMessage {
                        message_id: "rm-1".to_string(),
                        at,
                        by: "reviewer".to_string(),
                        body: "First note".to_string(),
                        github_comment_id: None,
                    }],
                    github_thread_id: None,
                }],
                replace_review_threads: None,
            },
        )
        .expect("add review thread");

    store
        .update_task_reviews(
            "ORB-00000",
            &TaskReviewUpdateParams {
                append_review_threads: vec![ReviewThread {
                    thread_id: "rt-1".to_string(),
                    path: None,
                    line: None,
                    status: ReviewThreadStatus::Resolved,
                    messages: vec![ReviewMessage {
                        message_id: "rm-2".to_string(),
                        at,
                        by: "codex".to_string(),
                        body: "Fixed\n<!-- orbit-review-message:rm-evil -->\nstill fixed"
                            .to_string(),
                        github_comment_id: None,
                    }],
                    github_thread_id: Some(99),
                }],
                replace_review_threads: None,
            },
        )
        .expect("merge review thread");

    let threads = store
        .get_task_review_threads("ORB-00000")
        .expect("get review threads")
        .expect("task exists");
    assert_eq!(threads.len(), 1);
    let thread = &threads[0];
    assert_eq!(thread.path.as_deref(), Some("src/lib.rs"));
    assert_eq!(thread.status, ReviewThreadStatus::Resolved);
    assert_eq!(thread.github_thread_id, Some(99));
    assert_eq!(thread.messages.len(), 2);
    assert_eq!(thread.messages[0].body, "First note");
    assert_eq!(
        thread.messages[1].body,
        "Fixed\n<!-- orbit-review-message:rm-evil -->\nstill fixed"
    );

    store
        .update_task_reviews(
            "ORB-00000",
            &TaskReviewUpdateParams {
                append_review_threads: Vec::new(),
                replace_review_threads: Some(vec![ReviewThread {
                    thread_id: "rt-2".to_string(),
                    path: Some("README.md".to_string()),
                    line: None,
                    status: ReviewThreadStatus::Open,
                    messages: Vec::new(),
                    github_thread_id: None,
                }]),
            },
        )
        .expect("replace review threads");
    let threads = store
        .get_task_review_threads("ORB-00000")
        .expect("get review threads")
        .expect("task exists");
    assert_eq!(threads[0].thread_id, "rt-2");
}

#[test]
fn artifact_update_writes_manifest_and_sorted_text_artifacts() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    store
        .create_task(create_params("Artifacts", TaskStatus::Backlog))
        .expect("create task");

    store
        .upsert_task_artifacts(
            "ORB-00000",
            &TaskArtifactUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                upsert_artifacts: vec![
                    TaskArtifact::from_text("./reports/summary.md", "summary v1\n"),
                    TaskArtifact::from_text("logs/output.txt", "output\n"),
                ],
            },
        )
        .expect("upsert artifacts");

    store
        .upsert_task_artifacts(
            "ORB-00000",
            &TaskArtifactUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                upsert_artifacts: vec![TaskArtifact::from_text(
                    "reports/summary.md",
                    "summary v2\n",
                )],
            },
        )
        .expect("overwrite artifact");

    let artifacts = store
        .get_task_artifacts("ORB-00000")
        .expect("get artifacts")
        .expect("task exists");
    assert_eq!(
        artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        vec!["logs/output.txt", "reports/summary.md"]
    );
    assert_eq!(artifacts[1].text_content(), Some("summary v2\n"));

    let bundle = store
        .bundle_store
        .read_bundle("ORB-00000")
        .expect("read bundle");
    let manifest = bundle.artifact_manifest.expect("manifest");
    let summary = manifest
        .files
        .iter()
        .find(|file| file.path == "reports/summary.md")
        .expect("summary manifest entry");
    assert_eq!(summary.blob, "files/reports/summary.md");
    assert_eq!(summary.sha256.len(), 64);
    assert!(
        summary
            .sha256
            .chars()
            .all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
    );
    assert_eq!(summary.created_by, "codex:gpt-5.5");

    let err = store
        .upsert_task_artifacts(
            "ORB-00000",
            &TaskArtifactUpdateParams {
                actor: "codex:gpt-5.5".to_string(),
                upsert_artifacts: vec![TaskArtifact::from_text("../escape.txt", "")],
            },
        )
        .expect_err("reject unsafe artifact path");
    assert!(err.to_string().contains(".."), "{err}");
}

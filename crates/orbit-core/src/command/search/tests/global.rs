use super::*;

#[test]
fn global_search_all_round_robins_total_limit_across_kinds() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let query = "fairlimit8";
    seed_search_fixture(&runtime, query, 20, 20, 20, 20);

    let response = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::All,
            limit: 8,
            ..Default::default()
        })
        .expect("search all");

    assert_eq!(response.results.len(), 8);
    for kind in ["task", "doc", "adr", "learning"] {
        let count = count_kind(&response.results, kind);
        assert!(
            (1..=3).contains(&count),
            "{kind} should contribute 1..=3 results, got {count}: {:?}",
            response.results
        );
    }
}

#[test]
fn global_search_all_limit_four_takes_one_from_each_kind() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let query = "fairlimit4";
    seed_search_fixture(&runtime, query, 20, 20, 20, 20);

    let response = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::All,
            limit: 4,
            ..Default::default()
        })
        .expect("search all");

    assert_eq!(response.results.len(), 4);
    for kind in ["task", "doc", "adr", "learning"] {
        assert_eq!(count_kind(&response.results, kind), 1, "{kind} count");
    }
}

#[test]
fn global_search_single_kind_limit_keeps_task_behavior() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let query = "fairtaskonly";
    seed_search_fixture(&runtime, query, 20, 20, 20, 20);

    let response = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::Task,
            limit: 8,
            ..Default::default()
        })
        .expect("search tasks");

    assert_eq!(response.results.len(), 8);
    assert!(response.results.iter().all(|hit| hit.kind == "task"));
}

#[test]
fn global_search_all_redistributes_when_one_kind_has_fewer_hits() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let query = "fairshortdoc";
    seed_search_fixture(&runtime, query, 20, 1, 20, 20);

    let response = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::All,
            limit: 12,
            ..Default::default()
        })
        .expect("search all");

    assert_eq!(response.results.len(), 12);
}

#[test]
fn global_search_all_preserves_in_kind_task_ranking() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let query = "fairrank";
    seed_search_fixture(&runtime, query, 20, 20, 20, 20);

    let task_only = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::Task,
            limit: 8,
            ..Default::default()
        })
        .expect("task branch");
    let merged = runtime
        .global_search(GlobalSearchParams {
            query: Some(query.to_string()),
            kind: GlobalSearchKind::All,
            limit: 8,
            ..Default::default()
        })
        .expect("merged search");

    let task_only_ids = task_only
        .results
        .iter()
        .map(|hit| hit.id.as_deref().expect("task id"))
        .collect::<Vec<_>>();
    let merged_task_ids = merged
        .results
        .iter()
        .filter(|hit| hit.kind == "task")
        .map(|hit| hit.id.as_deref().expect("task id"))
        .collect::<Vec<_>>();

    assert!(!merged_task_ids.is_empty());
    assert_eq!(
        merged_task_ids.as_slice(),
        &task_only_ids[..merged_task_ids.len()],
        "merged task hits should keep task-branch order"
    );
}

#[test]
fn global_search_path_filter_notes_doc_branch_skip() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    add_doc(&runtime, "docs/path-note.md", "needle path note");

    let response = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::All,
            path: Some("crates/orbit-cli/".to_string()),
            ..Default::default()
        })
        .expect("path search");

    assert!(
        response
            .notes
            .iter()
            .any(|note| note.contains("doc branch skipped") && note.contains("--path")),
        "notes should mention doc branch and --path: {:?}",
        response.notes
    );
}

#[test]
fn global_search_adr_tag_filter_matches_case_insensitive() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let adr_id = add_tagged_adr(&runtime);

    let response = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::Adr,
            tags: vec!["perf".to_string()],
            ..Default::default()
        })
        .expect("search by tag");

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].id.as_deref(), Some(adr_id.as_str()));
    assert_eq!(
        response.results[0].matched_by.as_deref(),
        Some(&["tag:perf".to_string()][..])
    );

    let negative = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::Adr,
            tags: vec!["security".to_string()],
            ..Default::default()
        })
        .expect("search by missing tag");
    assert!(negative.results.is_empty());
}

#[test]
fn global_search_adr_path_filter_matches_glob_containment() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let adr_id = add_tagged_adr(&runtime);

    let response = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::Adr,
            path: Some("crates/orbit-search/src/lib.rs".to_string()),
            ..Default::default()
        })
        .expect("search by path");

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].id.as_deref(), Some(adr_id.as_str()));
    assert_eq!(
        response.results[0].matched_by.as_deref(),
        Some(&["path:crates/orbit-search/src/lib.rs".to_string()][..])
    );

    let negative = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::Adr,
            path: Some("crates/orbit-core/src/lib.rs".to_string()),
            ..Default::default()
        })
        .expect("search by missing path");
    assert!(negative.results.is_empty());
}

#[test]
fn global_search_all_unions_adr_hits_for_tag_and_path_filters() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let adr_id = add_tagged_adr(&runtime);

    let response = runtime
        .global_search(GlobalSearchParams {
            kind: GlobalSearchKind::All,
            tags: vec!["perf".to_string()],
            path: Some("crates/orbit-search/src/lib.rs".to_string()),
            ..Default::default()
        })
        .expect("search all by tag and path");

    let adr_hit = response
        .results
        .iter()
        .find(|hit| hit.kind == "adr" && hit.id.as_deref() == Some(adr_id.as_str()))
        .expect("adr hit");
    assert_eq!(
        adr_hit.matched_by.as_deref(),
        Some(
            &[
                "tag:perf".to_string(),
                "path:crates/orbit-search/src/lib.rs".to_string(),
            ][..]
        )
    );
}

#[test]
fn global_search_status_filter_requires_kind_prefix() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let error = runtime
        .global_search(GlobalSearchParams {
            query: Some("needle".to_string()),
            status: vec!["open".to_string()],
            ..Default::default()
        })
        .expect_err("bare status token should fail");

    assert!(error.to_string().contains("`open`"));
    assert!(error.to_string().contains("kind:value"));
}

#[test]
fn global_search_status_filter_reports_invalid_kind_values() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_error = runtime
        .global_search(GlobalSearchParams {
            query: Some("needle".to_string()),
            status: vec!["task:not-a-status".to_string()],
            ..Default::default()
        })
        .expect_err("invalid task status should fail");
    assert!(task_error.to_string().contains("`not-a-status`"));
    assert!(task_error.to_string().contains("`task`"));

    let doc_error = runtime
        .global_search(GlobalSearchParams {
            query: Some("needle".to_string()),
            status: vec!["doc:proposed".to_string()],
            ..Default::default()
        })
        .expect_err("invalid doc status should fail");
    assert!(doc_error.to_string().contains("`proposed`"));
    assert!(doc_error.to_string().contains("`doc`"));
}

#[test]
fn global_search_status_filter_applies_per_kind_tokens() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let open_task = add_task_with_status(&runtime, "needle open task", TaskStatus::Backlog);
    let closed_task = add_task_with_status(&runtime, "needle closed task", TaskStatus::Done);
    add_doc(&runtime, "docs/status-active.md", "needle active doc");
    add_doc(&runtime, "docs/status-other.md", "unrelated summary");
    let proposed_adr = add_adr(&runtime, "needle proposed ADR", "## Context\n\nneedle.\n");
    let accepted_adr = add_adr(&runtime, "needle accepted ADR", "## Context\n\nneedle.\n");
    runtime
        .stores()
        .adrs()
        .update_status(&accepted_adr, AdrStatus::Accepted)
        .expect("accept adr");

    let response = runtime
        .global_search(GlobalSearchParams {
            query: Some("needle".to_string()),
            status: vec![
                "task:open".to_string(),
                "doc:active".to_string(),
                "adr:proposed".to_string(),
            ],
            ..Default::default()
        })
        .expect("search with per-kind status filters");
    let ids = response
        .results
        .iter()
        .filter_map(|hit| hit.id.as_deref())
        .collect::<Vec<_>>();
    let paths = response
        .results
        .iter()
        .filter_map(|hit| hit.path.as_deref())
        .collect::<Vec<_>>();

    assert!(ids.contains(&open_task.as_str()));
    assert!(!ids.contains(&closed_task.as_str()));
    assert!(ids.contains(&proposed_adr.as_str()));
    assert!(!ids.contains(&accepted_adr.as_str()));
    assert!(
        paths.contains(&"docs/status-active.md"),
        "expected active doc path in results: {:?}",
        response.results
    );
    assert!(!paths.contains(&"docs/status-other.md"));
}

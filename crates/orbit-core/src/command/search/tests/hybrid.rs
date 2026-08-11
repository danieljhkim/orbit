use super::*;
use serde_json::json;

#[test]
fn global_search_task_hybrid_preserves_retriever_breakdown() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_task_with_status(&runtime, "task hybrid breakdown", TaskStatus::Backlog);

    let response = with_task_semantic_override(Ok(vec![task_semantic_hit(&id, 0.75)]), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("task hybrid breakdown".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Task,
                limit: 3,
                ..Default::default()
            })
            .expect("task hybrid search")
    });

    assert_eq!(response.mode, GlobalSearchMode::Hybrid);
    assert_eq!(
        serde_json::to_value(&response.results[0]).expect("serialize hit"),
        json!({
            "kind": "task",
            "source": "semantic",
            "id": id,
            "status": "backlog",
            "best_field": "title",
            "snippet": "semantic task snippet",
            "score": 0.75,
            "score_breakdown": {
                "rrf": 0.75,
                "bm25_rank": 2,
                "cosine_rank": 1
            }
        })
    );
}

#[test]
fn global_search_task_hybrid_falls_back_to_lexical_on_semantic_error() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_task_with_status(&runtime, "task fallback needle", TaskStatus::Backlog);

    let response = with_task_semantic_override(Err("companion missing".to_string()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("task fallback needle".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Task,
                limit: 3,
                ..Default::default()
            })
            .expect("task lexical fallback")
    });

    assert_eq!(response.mode, GlobalSearchMode::Lexical);
    assert!(response.notes.iter().any(|note| {
        note.contains("falling back to lexical task search") && note.contains("companion missing")
    }));
    assert_eq!(response.results[0].source, "lexical");
    assert_eq!(response.results[0].id.as_deref(), Some(id.as_str()));
}

#[test]
fn global_search_doc_hybrid_uses_docs_semantic_weight() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    add_doc_with_tags(&runtime, "docs/z-lexical.md", "Literal primary", &["foo"]);
    add_doc(&runtime, "docs/y-lexical.md", "foo secondary");
    add_doc(&runtime, "docs/a-semantic.md", "Conceptual match");
    let semantic = vec![
        doc_semantic_hit("docs/a-semantic.md", 1.0),
        doc_semantic_hit("docs/y-lexical.md", 0.2),
    ];

    let top_path = |weight: f32| {
        fs::write(
            runtime.config_path(),
            format!("[docs.search]\nsemantic_weight = {weight:.1}\n"),
        )
        .expect("write config");
        with_doc_semantic_override(Ok(semantic.clone()), || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("foo".to_string()),
                    hybrid: true,
                    kind: GlobalSearchKind::Doc,
                    limit: 3,
                    ..Default::default()
                })
                .expect("doc hybrid search")
                .results
                .into_iter()
                .next()
                .expect("top result")
                .path
                .expect("doc path")
        })
    };

    assert_eq!(top_path(0.0), "docs/z-lexical.md");
    assert_eq!(top_path(1.0), "docs/a-semantic.md");
    assert_eq!(top_path(0.5), "docs/a-semantic.md");
}

#[test]
fn global_search_doc_hybrid_falls_back_to_lexical_on_semantic_error() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    add_doc_with_tags(
        &runtime,
        "docs/fallback-z-lexical.md",
        "Fallback primary",
        &["fallbackneedle"],
    );

    let response = with_doc_semantic_override(Err("companion missing".to_string()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("fallbackneedle".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Doc,
                limit: 3,
                ..Default::default()
            })
            .expect("fallback search")
    });

    assert!(
        response
            .notes
            .iter()
            .any(|note| note.contains("falling back to lexical"))
    );
    assert_eq!(response.results[0].source, "lexical");
    assert_eq!(
        response.results[0].path.as_deref(),
        Some("docs/fallback-z-lexical.md")
    );
}

#[test]
fn hybrid_handles_single_candidate_side() {
    let hit = GlobalSearchHit {
        kind: "doc".to_string(),
        source: "hybrid".to_string(),
        id: None,
        path: Some("docs/only.md".to_string()),
        title: None,
        summary: None,
        status: None,
        best_field: None,
        snippet: None,
        score: None,
        score_breakdown: None,
        matched_by: None,
    };
    let out = blend_doc_hybrid_candidates(
        vec![DocHybridCandidate {
            hit,
            lexical_score: Some(0.42),
            semantic_score: None,
            semantic: None,
        }],
        0.5,
    );

    assert!((out[0].score.expect("score") - 0.21).abs() < 0.0001);
}

#[test]
fn global_search_learning_lexical_mode_keeps_legacy_json_shape() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_learning_with(
        &runtime,
        "lexstable literal learning",
        &["lexstable"],
        Some(50),
    );

    let response =
        with_learning_semantic_override(Err("semantic should not be called".to_string()), || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("lexstable".to_string()),
                    kind: GlobalSearchKind::Learning,
                    limit: 5,
                    ..Default::default()
                })
                .expect("learning lexical search")
        });

    assert_eq!(response.mode, GlobalSearchMode::Lexical);
    assert_eq!(
        serde_json::to_value(&response.results).expect("serialize results"),
        json!([
            {
                "kind": "learning",
                "source": "lexical",
                "id": id,
                "summary": "lexstable literal learning",
                "status": "active",
                "matched_by": ["query:summary"]
            }
        ])
    );
}

#[test]
fn global_search_learning_hybrid_ranking_differs_from_lexical() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let semantic_id = add_learning(&runtime, "conceptual async-lock guidance");
    let lexical_id = add_learning_with(&runtime, "rankdiff literal foo guidance", &[], Some(100));

    let lexical = runtime
        .global_search(GlobalSearchParams {
            query: Some("rankdiff".to_string()),
            kind: GlobalSearchKind::Learning,
            limit: 2,
            ..Default::default()
        })
        .expect("learning lexical search");
    let hybrid = with_learning_semantic_override(
        Ok(vec![
            learning_semantic_hit(&semantic_id, 1.0),
            learning_semantic_hit(&lexical_id, 0.0),
        ]),
        || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("rankdiff".to_string()),
                    hybrid: true,
                    kind: GlobalSearchKind::Learning,
                    limit: 2,
                    ..Default::default()
                })
                .expect("learning hybrid search")
        },
    );

    assert_eq!(lexical.results[0].id.as_deref(), Some(lexical_id.as_str()));
    assert_eq!(hybrid.results[0].id.as_deref(), Some(semantic_id.as_str()));
    assert_ne!(lexical.results[0].id, hybrid.results[0].id);
}

#[test]
fn global_search_learning_hybrid_uses_learning_semantic_weight() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let semantic_id = add_learning(&runtime, "learningweight conceptual guidance");
    let lexical_id = add_learning_with(
        &runtime,
        "learningweight literal foo guidance",
        &[],
        Some(100),
    );
    let semantic = vec![
        learning_semantic_hit(&semantic_id, 1.0),
        learning_semantic_hit(&lexical_id, 0.0),
    ];

    let top_id = |weight: f32| {
        fs::write(
            runtime.config_path(),
            format!("[learning.search]\nsemantic_weight = {weight:.1}\n"),
        )
        .expect("write config");
        with_learning_semantic_override(Ok(semantic.clone()), || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("learningweight".to_string()),
                    hybrid: true,
                    kind: GlobalSearchKind::Learning,
                    limit: 2,
                    ..Default::default()
                })
                .expect("learning hybrid search")
                .results
                .into_iter()
                .next()
                .expect("top result")
                .id
                .expect("learning id")
        })
    };

    assert_eq!(top_id(0.0), lexical_id);
    assert_eq!(top_id(1.0), semantic_id);
    assert_eq!(top_id(0.5), semantic_id);
}

#[test]
fn global_search_learning_hybrid_falls_back_to_lexical_on_semantic_error() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_learning(&runtime, "learnfallback error literal");

    let response = with_learning_semantic_override(Err("companion missing".to_string()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("learnfallback".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Learning,
                limit: 3,
                ..Default::default()
            })
            .expect("fallback search")
    });

    assert!(
        response
            .notes
            .iter()
            .any(|note| note.contains("falling back to lexical"))
    );
    assert_eq!(response.results[0].source, "lexical");
    assert_eq!(response.results[0].id.as_deref(), Some(id.as_str()));
}

#[test]
fn global_search_learning_hybrid_falls_back_when_learning_embeddings_empty() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_learning(&runtime, "learnfallback empty literal");

    let response = with_learning_semantic_override(Ok(Vec::new()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("learnfallback".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Learning,
                limit: 3,
                ..Default::default()
            })
            .expect("fallback search")
    });

    assert!(
        response
            .notes
            .iter()
            .any(|note| note.contains("falling back to lexical"))
    );
    assert_eq!(response.results[0].source, "lexical");
    assert_eq!(response.results[0].id.as_deref(), Some(id.as_str()));
}

#[test]
fn learning_hybrid_handles_single_candidate_side() {
    let hit = GlobalSearchHit {
        kind: "learning".to_string(),
        source: "hybrid".to_string(),
        id: Some("L-0001".to_string()),
        path: None,
        title: None,
        summary: None,
        status: None,
        best_field: None,
        snippet: None,
        score: None,
        score_breakdown: None,
        matched_by: None,
    };
    let out = blend_learning_hybrid_candidates(
        vec![LearningHybridCandidate {
            hit,
            lexical_score: Some(0.42),
            semantic_score: None,
            semantic: None,
        }],
        0.5,
    );

    assert!((out[0].score.expect("score") - 0.21).abs() < 0.0001);
}

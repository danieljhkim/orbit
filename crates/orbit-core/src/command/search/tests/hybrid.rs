use super::*;
use serde_json::json;

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
fn global_search_doc_hybrid_preserves_adr_lexical_hits() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let adr_id = add_adr(&runtime, "foo ADR", "## Context\n\nBody.\n");
    let lexical_adr = runtime
        .search_docs("foo", Some(5), true)
        .expect("direct docs search")
        .into_iter()
        .find_map(|result| match result {
            SearchResult::Adr(result) => Some(result),
            SearchResult::Doc(_) => None,
        })
        .expect("lexical adr");

    let response = with_doc_semantic_override(Ok(Vec::new()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("foo".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Doc,
                limit: 5,
                ..Default::default()
            })
            .expect("hybrid doc search")
    });
    let adr_hit = response
        .results
        .iter()
        .find(|hit| hit.kind == "adr")
        .expect("adr hit");

    assert_eq!(adr_hit.id.as_deref(), Some(adr_id.as_str()));
    assert_eq!(adr_hit.source, "lexical");
    assert_eq!(adr_hit.score, Some(lexical_adr.score as f32));
}

#[test]
fn global_search_adr_lexical_mode_keeps_legacy_json_shape() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_adr(
        &runtime,
        "lexadrstable literal ADR",
        "## Decision\nBody should not affect lexical shape.\n",
    );

    let response =
        with_adr_semantic_override(Err("semantic should not be called".to_string()), || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("lexadrstable".to_string()),
                    kind: GlobalSearchKind::Adr,
                    limit: 5,
                    ..Default::default()
                })
                .expect("ADR lexical search")
        });

    assert_eq!(response.mode, GlobalSearchMode::Lexical);
    assert_eq!(
        serde_json::to_value(&response.results).expect("serialize results"),
        json!([
            {
                "kind": "adr",
                "source": "lexical",
                "id": id,
                "path": format!(".orbit/adrs/proposed/{id}/body.md"),
                "title": "lexadrstable literal ADR",
                "status": "proposed",
                "score": 92.0,
                "matched_by": ["title"]
            }
        ])
    );
}

#[test]
fn global_search_adr_hybrid_ranking_differs_from_lexical() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let semantic_id = add_adr(
        &runtime,
        "Conceptual async-lock ADR",
        "## Decision\nUse scoped guards for await boundaries.\n",
    );
    let lexical_id = add_adr(
        &runtime,
        "rankadrfoo literal ADR",
        "## Decision\nLiteral match only.\n",
    );
    add_adr(
        &runtime,
        "rankadrfoo secondary ADR",
        "## Decision\nSecond literal match.\n",
    );

    let lexical = runtime
        .global_search(GlobalSearchParams {
            query: Some("rankadrfoo".to_string()),
            kind: GlobalSearchKind::Adr,
            limit: 2,
            ..Default::default()
        })
        .expect("ADR lexical search");
    let hybrid = with_adr_semantic_override(
        Ok(vec![
            adr_semantic_hit(&semantic_id, 1.0),
            adr_semantic_hit(&lexical_id, 0.0),
        ]),
        || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("rankadrfoo".to_string()),
                    hybrid: true,
                    kind: GlobalSearchKind::Adr,
                    limit: 2,
                    ..Default::default()
                })
                .expect("ADR hybrid search")
        },
    );

    assert_eq!(lexical.results[0].id.as_deref(), Some(lexical_id.as_str()));
    assert_eq!(hybrid.results[0].id.as_deref(), Some(semantic_id.as_str()));
    assert_ne!(lexical.results[0].id, hybrid.results[0].id);
}

#[test]
fn global_search_doc_hybrid_ranks_federated_adr_semantic_hits() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    add_doc(&runtime, "docs/fedadr-lexical.md", "fedadr lexical doc");
    let adr_id = add_adr(
        &runtime,
        "Federated semantic ADR",
        "## Decision\nConceptual match without the literal query.\n",
    );
    fs::write(
        runtime.config_path(),
        "[docs.search]\nsemantic_weight = 1.0\n[adr.search]\nsemantic_weight = 1.0\n",
    )
    .expect("write config");

    let lexical = runtime
        .global_search(GlobalSearchParams {
            query: Some("fedadr".to_string()),
            kind: GlobalSearchKind::Doc,
            limit: 3,
            ..Default::default()
        })
        .expect("doc lexical search");
    let hybrid = with_doc_semantic_override(
        Ok(vec![doc_semantic_hit("docs/fedadr-lexical.md", 0.0)]),
        || {
            with_adr_semantic_override(Ok(vec![adr_semantic_hit(&adr_id, 1.0)]), || {
                runtime
                    .global_search(GlobalSearchParams {
                        query: Some("fedadr".to_string()),
                        hybrid: true,
                        kind: GlobalSearchKind::Doc,
                        limit: 3,
                        ..Default::default()
                    })
                    .expect("doc hybrid search")
            })
        },
    );

    assert_eq!(lexical.results[0].kind, "doc");
    assert_eq!(hybrid.results[0].kind, "adr");
    assert_eq!(hybrid.results[0].id.as_deref(), Some(adr_id.as_str()));
}

#[test]
fn global_search_adr_hybrid_uses_adr_semantic_weight() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let semantic_id = add_adr(
        &runtime,
        "ADR weight conceptual",
        "## Decision\nConceptual guidance.\n",
    );
    let lexical_id = add_adr(
        &runtime,
        "adrweight literal ADR",
        "## Decision\nLiteral guidance.\n",
    );
    add_adr(
        &runtime,
        "adrweight secondary ADR",
        "## Decision\nSecond literal guidance.\n",
    );
    let semantic = vec![
        adr_semantic_hit(&semantic_id, 1.0),
        adr_semantic_hit(&lexical_id, 0.0),
    ];

    let top_id = |weight: f32| {
        fs::write(
            runtime.config_path(),
            format!("[adr.search]\nsemantic_weight = {weight:.1}\n"),
        )
        .expect("write config");
        with_adr_semantic_override(Ok(semantic.clone()), || {
            runtime
                .global_search(GlobalSearchParams {
                    query: Some("adrweight".to_string()),
                    hybrid: true,
                    kind: GlobalSearchKind::Adr,
                    limit: 2,
                    ..Default::default()
                })
                .expect("ADR hybrid search")
                .results
                .into_iter()
                .next()
                .expect("top result")
                .id
                .expect("ADR id")
        })
    };

    assert_eq!(top_id(0.0), lexical_id);
    assert_eq!(top_id(1.0), semantic_id);
    assert_eq!(top_id(0.5), semantic_id);
}

#[test]
fn global_search_adr_hybrid_falls_back_to_lexical_on_semantic_error() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_adr(
        &runtime,
        "adrfallback error literal",
        "## Decision\nBody.\n",
    );

    let response = with_adr_semantic_override(Err("companion missing".to_string()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("adrfallback".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Adr,
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
fn global_search_adr_hybrid_falls_back_when_adr_embeddings_empty() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let id = add_adr(
        &runtime,
        "adrfallback empty literal",
        "## Decision\nBody.\n",
    );

    let response = with_adr_semantic_override(Ok(Vec::new()), || {
        runtime
            .global_search(GlobalSearchParams {
                query: Some("adrfallback".to_string()),
                hybrid: true,
                kind: GlobalSearchKind::Adr,
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
fn adr_hybrid_handles_single_candidate_side() {
    let hit = GlobalSearchHit {
        kind: "adr".to_string(),
        source: "hybrid".to_string(),
        id: Some("ADR-0001".to_string()),
        path: Some(".orbit/adrs/proposed/ADR-0001/body.md".to_string()),
        title: None,
        summary: None,
        status: None,
        best_field: None,
        snippet: None,
        score: None,
        matched_by: None,
    };
    let out = blend_adr_hybrid_candidates(
        vec![AdrHybridCandidate {
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

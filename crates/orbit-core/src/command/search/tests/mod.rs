use std::fs;

use orbit_common::types::{AdrStatus, LearningScope, TaskPriority, TaskStatus, TaskType};
use orbit_search::{AdrSemanticHit, DocSemanticHit, LearningSemanticHit};
use orbit_store::{AdrCreateParams, LearningCreateParams, TaskCreateParams};

use super::*;
use crate::{OrbitRuntime, SearchResult};

mod global;
mod hybrid;
mod path_match;
mod types;

fn add_tagged_adr(runtime: &OrbitRuntime) -> String {
    add_adr(runtime, "ADR tag path bridge", "## Context\n\nTest.\n")
}

fn add_adr(runtime: &OrbitRuntime, title: &str, body: &str) -> String {
    runtime
        .stores()
        .adrs()
        .add(AdrCreateParams {
            title: title.to_string(),
            owner: "codex".to_string(),
            related_features: Vec::new(),
            related_tasks: Vec::new(),
            tags: vec!["Perf".to_string(), "orbit-search".to_string()],
            paths: vec!["crates/orbit-search/**".to_string()],
            body: body.to_string(),
        })
        .expect("add adr")
        .id
}

fn add_task_with_status(runtime: &OrbitRuntime, title: &str, status: TaskStatus) -> String {
    runtime
        .stores()
        .tasks()
        .create(TaskCreateParams {
            actor: "test".to_string(),
            parent_id: None,
            title: title.to_string(),
            description: "needle task body".to_string(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            relations: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            workspace_path: Some(runtime.paths().repo_root.to_string_lossy().into_owned()),
            repo_root: None,
            created_by: Some("test".to_string()),
            planned_by: None,
            implemented_by: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: None,
            comments: Vec::new(),
        })
        .expect("create task")
        .id
}

fn add_doc(runtime: &OrbitRuntime, path: &str, summary: &str) {
    add_doc_with_tags(runtime, path, summary, &[]);
}

fn add_doc_with_tags(runtime: &OrbitRuntime, path: &str, summary: &str, tags: &[&str]) {
    let doc_path = runtime.paths().repo_root.join(path);
    fs::create_dir_all(doc_path.parent().expect("doc parent")).expect("create doc parent");
    let tags_line = if tags.is_empty() {
        String::new()
    } else {
        format!("tags: [{}]\n", tags.join(", "))
    };
    fs::write(
        doc_path,
        format!("---\ntype: context\nsummary: {summary}\n{tags_line}---\n\nneedle doc body\n"),
    )
    .expect("write doc");
}

fn add_learning(runtime: &OrbitRuntime, summary: &str) -> String {
    add_learning_with(runtime, summary, &[], None)
}

fn add_learning_with(
    runtime: &OrbitRuntime,
    summary: &str,
    tags: &[&str],
    priority: Option<u8>,
) -> String {
    runtime
        .create_learning(LearningCreateParams {
            summary: summary.to_string(),
            scope: LearningScope {
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                ..Default::default()
            },
            body: format!("{summary} body"),
            evidence: Vec::new(),
            created_by: Some("test".to_string()),
            priority,
        })
        .expect("add learning")
        .id
}

// L-0026: keep each caller's query unique; in-memory doc files share the temp parent.
fn seed_search_fixture(
    runtime: &OrbitRuntime,
    query: &str,
    task_count: usize,
    doc_count: usize,
    adr_count: usize,
    learning_count: usize,
) {
    for index in 0..task_count {
        add_task_with_status(
            runtime,
            &format!("{query} task {index:02}"),
            TaskStatus::Backlog,
        );
    }
    for index in 0..doc_count {
        add_doc(
            runtime,
            &format!("docs/{query}-doc-{index:02}.md"),
            &format!("{query} doc {index:02}"),
        );
    }
    for index in 0..adr_count {
        add_adr(
            runtime,
            &format!("{query} ADR {index:02}"),
            &format!("## Context\n\n{query} adr body.\n"),
        );
    }
    for index in 0..learning_count {
        add_learning(runtime, &format!("{query} learning {index:02}"));
    }
}

fn count_kind(results: &[GlobalSearchHit], kind: &str) -> usize {
    results.iter().filter(|hit| hit.kind == kind).count()
}

fn doc_semantic_hit(path: &str, score: f32) -> DocSemanticHit {
    DocSemanticHit {
        source_id: path.to_string(),
        best_field: "body".to_string(),
        snippet: "semantic snippet".to_string(),
        score,
    }
}

fn adr_semantic_hit(id: &str, score: f32) -> AdrSemanticHit {
    AdrSemanticHit {
        source_id: id.to_string(),
        best_field: "decision".to_string(),
        snippet: "semantic ADR snippet".to_string(),
        score,
    }
}

fn learning_semantic_hit(id: &str, score: f32) -> LearningSemanticHit {
    LearningSemanticHit {
        source_id: id.to_string(),
        best_field: "summary".to_string(),
        snippet: "semantic learning snippet".to_string(),
        score,
    }
}

fn with_doc_semantic_override<T>(
    result: Result<Vec<DocSemanticHit>, String>,
    f: impl FnOnce() -> T,
) -> T {
    DOC_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(result);
    });
    let out = f();
    DOC_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    out
}

fn with_adr_semantic_override<T>(
    result: Result<Vec<AdrSemanticHit>, String>,
    f: impl FnOnce() -> T,
) -> T {
    ADR_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(result);
    });
    let out = f();
    ADR_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    out
}

fn with_learning_semantic_override<T>(
    result: Result<Vec<LearningSemanticHit>, String>,
    f: impl FnOnce() -> T,
) -> T {
    LEARNING_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(result);
    });
    let out = f();
    LEARNING_SEMANTIC_SEARCH_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    out
}

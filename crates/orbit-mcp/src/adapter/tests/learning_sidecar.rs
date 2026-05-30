use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{LearningInjectionCaps, LearningInjectionState};
use serde_json::{Value, json};

use super::super::test_support::{LearningSidecarHost, request_with_args};
use super::super::{OrbitToolServer, PROCESS_LEARNING_SESSION_KEY};

#[tokio::test]
async fn learning_sidecar_present_with_summary_only_on_path_match() {
    let mut search_by_path = HashMap::new();
    search_by_path.insert(
        "crates/orbit-engine/src/lib.rs".to_string(),
        vec![json!({
            "id": "L-0001",
            "summary": "Remember the engine rule.",
            "body": "full body must stay out",
            "updated_at": "2026-05-15T00:00:00Z",
            "priority": 7
        })],
    );
    let host = Arc::new(LearningSidecarHost::new(
        json!({
            "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
        }),
        search_by_path,
    ));
    let server = OrbitToolServer::new_for_test(
        host,
        None,
        LearningInjectionCaps::default(),
        LearningInjectionState::default(),
    );

    let result = server
        .call_tool_request(request_with_args(
            "orbit.task.show",
            json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
        ))
        .await
        .expect("call succeeds");
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");

    assert_eq!(
        structured.get("learnings"),
        Some(&json!([{
            "id": "L-0001",
            "summary": "Remember the engine rule."
        }]))
    );
    assert!(
        !serde_json::to_string(structured)
            .expect("json")
            .contains("full body")
    );
}

#[tokio::test]
async fn learning_sidecar_absent_when_no_learning_matches() {
    let mut search_by_path = HashMap::new();
    search_by_path.insert("crates/orbit-engine/src/lib.rs".to_string(), Vec::new());
    let host = Arc::new(LearningSidecarHost::new(
        json!({
            "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
        }),
        search_by_path,
    ));
    let server = OrbitToolServer::new_for_test(
        host,
        None,
        LearningInjectionCaps::default(),
        LearningInjectionState::default(),
    );

    let result = server
        .call_tool_request(request_with_args(
            "orbit.task.show",
            json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
        ))
        .await
        .expect("call succeeds");
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");

    assert!(structured.get("learnings").is_none());
}

#[tokio::test]
async fn l1_seeded_learning_is_suppressed_by_l2_dedup_state() {
    let mut search_by_path = HashMap::new();
    search_by_path.insert(
        "crates/orbit-engine/src/lib.rs".to_string(),
        vec![json!({
            "id": "L-0001",
            "summary": "Already injected at L1.",
            "updated_at": "2026-05-15T00:00:00Z",
            "priority": null
        })],
    );
    let host = Arc::new(LearningSidecarHost::new(
        json!({
            "context_files": ["file:crates/orbit-engine/src/lib.rs"]
        }),
        search_by_path,
    ));
    let initial_state = LearningInjectionState::seeded(["L-0001".to_string()]);
    let server =
        OrbitToolServer::new_for_test(host, None, LearningInjectionCaps::default(), initial_state);

    let result = server
        .call_tool_request(request_with_args(
            "orbit.task.show",
            json!({"id": "ORB-00009"}),
        ))
        .await
        .expect("call succeeds");
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");

    assert!(structured.get("learnings").is_none());
    let states = server.learning_states.lock().await;
    let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
    assert_eq!(state.count, 1);
    assert!(state.emitted_ids.contains("L-0001"));
}

#[tokio::test]
async fn learning_sidecar_enforces_per_session_hard_cap() {
    let mut search_by_path = HashMap::new();
    for call_idx in 0..5 {
        let path = format!("p{call_idx}.rs");
        let rows: Vec<_> = (0..5)
            .map(|row_idx| {
                let id_idx = call_idx * 5 + row_idx;
                json!({
                    "id": format!("L-{id_idx:04}"),
                    "summary": format!("Learning {id_idx}"),
                    "updated_at": "2026-05-15T00:00:00Z",
                    "priority": null
                })
            })
            .collect();
        search_by_path.insert(path, rows);
    }
    let host = Arc::new(LearningSidecarHost::new(json!({}), search_by_path));
    let server = OrbitToolServer::new_for_test(
        host,
        None,
        LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        },
        LearningInjectionState::default(),
    );
    let mut emitted = 0usize;

    for call_idx in 0..5 {
        let result = server
            .call_tool_request(request_with_args(
                "orbit.task.show",
                json!({"selector": format!("file:p{call_idx}.rs")}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");
        emitted += structured
            .get("learnings")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
    }

    assert_eq!(emitted, 20);
    let states = server.learning_states.lock().await;
    let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
    assert_eq!(state.count, 20);
    assert_eq!(state.emitted_ids.len(), 20);
}

#[tokio::test]
async fn learning_sidecar_session_id_persists_admission_through_host_state() {
    let mut search_by_path = HashMap::new();
    search_by_path.insert(
        "crates/orbit-engine/src/lib.rs".to_string(),
        vec![json!({
            "id": "L-0001",
            "summary": "Persisted through host state.",
            "updated_at": "2026-05-15T00:00:00Z",
            "priority": null
        })],
    );
    let host = Arc::new(LearningSidecarHost::new(
        json!({
            "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
        }),
        search_by_path,
    ));
    let caps = LearningInjectionCaps {
        per_call: 5,
        per_session_hard: 20,
    };
    let server = OrbitToolServer::new_for_test(
        host.clone(),
        Some("session-1".to_string()),
        caps,
        LearningInjectionState::default(),
    );

    let result = server
        .call_tool_request(request_with_args(
            "orbit.task.show",
            json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
        ))
        .await
        .expect("first call succeeds");
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");
    assert_eq!(
        structured.get("learnings"),
        Some(&json!([{
            "id": "L-0001",
            "summary": "Persisted through host state."
        }]))
    );

    let server = OrbitToolServer::new_for_test(
        host,
        Some("session-1".to_string()),
        caps,
        LearningInjectionState::default(),
    );
    let result = server
        .call_tool_request(request_with_args(
            "orbit.task.show",
            json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
        ))
        .await
        .expect("second call succeeds");
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");
    assert!(structured.get("learnings").is_none());
}

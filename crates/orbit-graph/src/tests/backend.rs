use serde_json::json;

use crate::{GraphBackend, GraphQueryKind, route_query};

#[test]
fn backend_resolver_uses_override_then_env_then_legacy_default() {
    assert_eq!(
        GraphBackend::resolve_from(None, None).unwrap(),
        GraphBackend::Legacy
    );
    assert_eq!(
        GraphBackend::resolve_from(None, Some("new".to_string())).unwrap(),
        GraphBackend::New
    );
    assert_eq!(
        GraphBackend::resolve_from(Some(GraphBackend::Both), Some("new".to_string())).unwrap(),
        GraphBackend::Both
    );
}

#[test]
fn backend_resolver_rejects_unknown_env_value() {
    let error = GraphBackend::resolve_from(None, Some("sideways".to_string()))
        .expect_err("invalid backend rejected");
    assert!(error.to_string().contains("expected legacy, new, or both"));
}

#[test]
fn route_query_uses_new_backend_when_requested() {
    let value = route_query::<String, _, _>(
        GraphBackend::New,
        GraphQueryKind::Search,
        || Ok(json!({ "backend": "new" })),
        || Ok(json!({ "backend": "legacy" })),
    )
    .unwrap();

    assert_eq!(value["backend"], "new");
}

#[test]
fn route_query_both_returns_new_when_shadow_fails() {
    let value = route_query(
        GraphBackend::Both,
        GraphQueryKind::Refs,
        || Ok(json!({ "backend": "new" })),
        || Err("shadow unavailable".to_string()),
    )
    .unwrap();

    assert_eq!(value["backend"], "new");
}

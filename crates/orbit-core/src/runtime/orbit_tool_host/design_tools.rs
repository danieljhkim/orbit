use orbit_common::types::{
    OrbitError, normalize_optional_attribution_label, optional_string, required_string,
};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::design;

pub(super) fn init(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let feature = required_string(&input, &["feature"], "feature")?;
    let owner = optional_string(&input, "owner")?
        .unwrap_or_else(|| actor_label(runtime, agent.as_deref(), model.as_deref()));
    let workspace = workspace_root(&input)?;
    let summary = design::init_feature(&workspace, &feature, &owner)?;
    serde_json::to_value(summary)
        .map_err(|error| OrbitError::Execution(format!("serialize design init response: {error}")))
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let _ = runtime;
    let workspace = workspace_root(&input)?;
    let features = design::list_features(&workspace)?;
    serde_json::to_value(features)
        .map_err(|error| OrbitError::Execution(format!("serialize design list response: {error}")))
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let _ = runtime;
    let feature = required_string(&input, &["feature"], "feature")?;
    let workspace = workspace_root(&input)?;
    let summary = design::show_feature(&workspace, &feature)?;
    serde_json::to_value(summary)
        .map_err(|error| OrbitError::Execution(format!("serialize design show response: {error}")))
}

fn actor_label(runtime: &OrbitRuntime, agent: Option<&str>, model: Option<&str>) -> String {
    normalize_optional_attribution_label(model.or(agent), model)
        .unwrap_or_else(|| runtime.actor_label().to_string())
}

fn workspace_root(input: &Value) -> Result<std::path::PathBuf, OrbitError> {
    if let Some(workspace) = optional_string(input, "workspace")? {
        return Ok(std::path::PathBuf::from(workspace));
    }
    std::env::current_dir().map_err(|error| OrbitError::Io(error.to_string()))
}

#[cfg(test)]
mod tests {
    use orbit_common::types::NotFoundKind;
    use serde_json::json;

    use super::*;
    use crate::runtime::orbit_tool_host::test_support::test_runtime;

    #[test]
    fn init_scaffolds_feature_and_show_returns_doc_metadata() {
        let (_guard, runtime, repo_root) = test_runtime();
        let created = init(
            &runtime,
            json!({
                "feature": "design-docs",
                "owner": "codex",
                "workspace": repo_root.to_string_lossy(),
            }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("init design docs");

        assert_eq!(created["feature"], "design-docs");
        assert!(
            repo_root
                .join("docs/design/design-docs/specs")
                .read_dir()
                .expect("read specs")
                .next()
                .is_none()
        );
        assert_eq!(created["docs"]["1_overview.md"]["owner"], json!("codex"));
        assert_eq!(
            created["docs"]["1_overview.md"]["decay_status"],
            json!("fresh")
        );

        let listed =
            list(&runtime, json!({"workspace": repo_root.to_string_lossy()})).expect("list");
        let features = listed.as_array().expect("array");
        assert!(
            features
                .iter()
                .any(|feature| feature["feature"] == "design-docs")
        );
    }

    #[test]
    fn show_missing_feature_returns_typed_not_found() {
        let (_guard, runtime, repo_root) = test_runtime();
        let error = show(
            &runtime,
            json!({"feature": "missing-feature", "workspace": repo_root.to_string_lossy()}),
        )
        .expect_err("missing");
        assert!(matches!(
            error,
            OrbitError::NotFound {
                kind: NotFoundKind::DesignFeature,
                ..
            }
        ));
    }
}

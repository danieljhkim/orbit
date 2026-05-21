#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use orbit_tools::ToolRegistry;

#[test]
fn unused_tools_are_not_registered_in_public_surface() {
    let names = registered_tool_names();

    for removed in [
        "fs.copy",
        "fs.create",
        "fs.ls",
        "fs.mkdir",
        "fs.move",
        "fs.patch",
        "fs.write",
        "git.commit",
        "git.stage_paths",
        "github.auth.status",
        "github.pr.checkout",
        "github.pr.checks",
        "github.pr.close",
        "github.pr.list",
        "github.repo.view",
        "net.http",
        "orbit.groundhog.checkpoint_deviate",
        "proc.which",
        "time.now",
        "time.sleep",
    ] {
        assert!(
            !names.contains(removed),
            "removed tool still registered: {removed}"
        );
    }

    let removed_prefix = "orbit.semantic.";
    for removed in ["related", "search"] {
        let name = format!("{removed_prefix}{removed}");
        assert!(
            !names.contains(name.as_str()),
            "removed tool still registered: {name}"
        );
    }
}

#[test]
fn workflow_critical_tools_remain_registered() {
    let names = registered_tool_names();

    for retained in [
        "fs.read",
        "fs.delete",
        "git.push",
        "github.pr.comment",
        "github.pr.comment.reply",
        "github.pr.comments",
        "github.pr.create",
        "github.pr.merge",
        "github.pr.review",
        "github.pr.review.comment",
        "github.pr.view",
        "orbit.graph.callers",
        "orbit.graph.deps",
        "orbit.graph.history",
        "orbit.graph.implementors",
        "orbit.graph.overview",
        "orbit.graph.pack",
        "orbit.graph.refs",
        "orbit.graph.search",
        "orbit.graph.show",
        "orbit.groundhog.checkpoint_failure",
        "orbit.groundhog.checkpoint_success",
        "orbit.groundhog.side_effect",
        "orbit.pipeline.invoke",
        "orbit.pipeline.wait",
        "orbit.search",
        "orbit.semantic.index",
        "orbit.semantic.install",
        "orbit.semantic.stats",
        "orbit.semantic.uninstall",
        "orbit.task.artifact.put",
        "proc.spawn",
    ] {
        assert!(
            names.contains(retained),
            "workflow-critical tool missing: {retained}"
        );
    }
}

#[test]
fn global_search_schema_drops_retired_semantic_tuning_params() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let schema = registry
        .get_schema("orbit.search")
        .expect("global search schema");
    let names = schema
        .parameters
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();

    assert!(!names.contains(&"field"));
    assert!(!names.contains(&"embedding_model"));
}

#[test]
fn friction_surface_supports_artifact_triage() {
    let names = registered_tool_names();

    for retained in [
        "orbit.friction.add",
        "orbit.friction.list",
        "orbit.friction.resolve",
        "orbit.friction.show",
        "orbit.friction.stats",
        "orbit.friction.tags",
        "orbit.friction.update",
    ] {
        assert!(
            names.contains(retained),
            "friction tool missing: {retained}"
        );
    }

    for removed in ["orbit.friction.delete", "orbit.friction.reject"] {
        assert!(
            !names.contains(removed),
            "destructive friction tool registered: {removed}"
        );
    }
}

#[test]
fn task_dependency_params_remain_in_agent_tool_schemas() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    for tool_name in ["orbit.task.add", "orbit.task.update"] {
        let schema = registry
            .get_schema(tool_name)
            .unwrap_or_else(|| panic!("{tool_name} schema"));
        let dependency_param = schema
            .parameters
            .iter()
            .find(|param| param.name == "dependencies")
            .unwrap_or_else(|| panic!("{tool_name} dependencies param"));

        assert_eq!(dependency_param.param_type, "string_list");
        assert!(!dependency_param.required);
        assert!(
            schema.parameters.iter().any(|param| param.name == "crew"),
            "{tool_name} should expose crew"
        );
    }
}

#[test]
fn task_add_update_schemas_use_model_only_identity() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    for tool_name in ["orbit.task.add", "orbit.task.update"] {
        let schema = registry
            .get_schema(tool_name)
            .unwrap_or_else(|| panic!("{tool_name} schema"));
        assert!(
            schema.parameters.iter().any(|param| param.name == "model"),
            "{tool_name} should expose model attribution"
        );
        assert!(
            schema.parameters.iter().all(|param| param.name != "agent"),
            "{tool_name} should not expose agent attribution"
        );
    }
}

#[test]
fn task_delete_schema_exposes_optional_force_boolean() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let schema = registry
        .get_schema("orbit.task.delete")
        .expect("task delete schema");
    let force_param = schema
        .parameters
        .iter()
        .find(|param| param.name == "force")
        .expect("force param");

    assert_eq!(force_param.param_type, "boolean");
    assert!(!force_param.required);
}

fn registered_tool_names() -> BTreeSet<String> {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
        .schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect()
}

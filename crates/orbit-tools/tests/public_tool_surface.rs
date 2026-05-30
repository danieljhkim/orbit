#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use orbit_common::types::RETIRED_TASK_ADD_INPUT_FIELDS;
use orbit_tools::ToolRegistry;

const INACTIVE_TOOL_NAMES: &[&str] = &[
    "orbit.docs.index",
    "orbit.docs.migrate",
    "orbit.docs.add",
    "orbit.docs.list",
    "orbit.docs.show",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.semantic.index",
    "orbit.semantic.install",
    "orbit.semantic.stats",
    "orbit.graph.history",
    "orbit.learning.sync",
    "orbit.learning.list",
    "orbit.friction.stats",
    // ORB-00289: admin/destructive ops — CLI path retains them, agent
    // MCP surface does not expose them.
    "orbit.adr.list",
    "orbit.semantic.uninstall",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.learning.comment.delete",
    "orbit.learning.prune",
];

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

    let removed_docs_reindex = ["orbit.docs", "reindex"].join(".");
    assert!(
        !names.contains(removed_docs_reindex.as_str()),
        "removed docs reindex tool still registered"
    );
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
        // ORB-00289: `orbit.semantic.uninstall` is inactive on the agent
        // surface; its inactive-classification is covered by
        // `inactive_ops_tools_*` and `INACTIVE_TOOL_NAMES` above.
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
fn inactive_ops_tools_are_hidden_from_default_registry_surface() {
    let names = registered_tool_names();

    for inactive in INACTIVE_TOOL_NAMES {
        assert!(
            !names.contains(*inactive),
            "inactive tool must be hidden from default registry schemas: {inactive}"
        );
    }
}

#[test]
fn inactive_ops_tools_remain_auditable_in_full_registry_surface() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    let all_names = registry
        .all_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect::<BTreeSet<_>>();

    for inactive in INACTIVE_TOOL_NAMES {
        assert!(
            all_names.contains(*inactive),
            "inactive tool must remain registered for inspection: {inactive}"
        );
        assert!(
            !registry.is_active(inactive),
            "inactive tool must be marked inactive in the registry: {inactive}"
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
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    let active: BTreeSet<String> = registry
        .schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    let all: BTreeSet<String> = registry
        .all_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();

    for retained in [
        "orbit.friction.add",
        "orbit.friction.tags",
        "orbit.friction.update",
    ] {
        assert!(
            active.contains(retained),
            "agent-facing friction tool missing from active surface: {retained}"
        );
    }

    for removed in ["orbit.friction.delete", "orbit.friction.reject"] {
        assert!(
            !all.contains(removed),
            "destructive friction tool registered: {removed}"
        );
    }

    // Triage surface (list/show/resolve) and stats are CLI / dashboard only:
    // registered for `runtime.run_tool` but hidden from the default agent
    // surface.
    for cli_only in [
        "orbit.friction.list",
        "orbit.friction.resolve",
        "orbit.friction.show",
        "orbit.friction.stats",
    ] {
        assert!(
            !active.contains(cli_only),
            "{cli_only} must stay hidden from the default registry surface"
        );
        assert!(
            all.contains(cli_only),
            "{cli_only} must remain reachable via `runtime.run_tool`"
        );
    }
}

#[test]
fn task_add_schema_uses_trimmed_authoring_surface() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let schema = registry
        .get_schema("orbit.task.add")
        .expect("orbit.task.add schema");
    let names = schema
        .parameters
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "title",
            "description",
            "workspace",
            "acceptance_criteria",
            "tags",
            "context_files",
            "priority",
            "complexity",
            "type",
            "relations",
            "model",
        ]
    );
    for removed in RETIRED_TASK_ADD_INPUT_FIELDS {
        assert!(
            !names.contains(removed),
            "orbit.task.add schema must not expose retired param {removed}"
        );
    }
}

#[test]
fn task_update_dependency_params_remain_in_agent_tool_schema() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let schema = registry
        .get_schema("orbit.task.update")
        .expect("orbit.task.update schema");
    let dependency_param = schema
        .parameters
        .iter()
        .find(|param| param.name == "dependencies")
        .expect("orbit.task.update dependencies param");

    assert_eq!(dependency_param.param_type, "string_list");
    assert!(!dependency_param.required);
    assert!(
        schema.parameters.iter().any(|param| param.name == "crew"),
        "orbit.task.update should expose crew"
    );
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

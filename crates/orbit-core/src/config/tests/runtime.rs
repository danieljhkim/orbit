use super::super::ConfigSnapshot;
use super::super::runtime::*;
use orbit_common::types::{Crew, CrewAssignment, OrbitError};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::tempdir;

fn write_config(dir: &Path, body: &str) {
    std::fs::write(dir.join("config.toml"), body).expect("write config");
}

fn single_family_crew(name: &str) -> Crew {
    let assignment = CrewAssignment {
        model: format!("{name}-model"),
        provider: name.to_string(),
    };
    Crew {
        name: name.to_string(),
        assignment,
        description: None,
        tags: Vec::new(),
    }
}

#[test]
fn built_in_crews_use_standard_model_specific_names() {
    let crews = default_crews();
    assert_eq!(
        crews.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "fable", "gemini", "grok", "luna", "opus", "sol", "sonnet", "terra"
        ]
    );
    for (name, provider, model) in [
        ("opus", "claude", "opus"),
        ("sonnet", "claude", "sonnet"),
        ("fable", "claude", "fable"),
        ("sol", "codex", "gpt-5.6-sol"),
        ("terra", "codex", "gpt-5.6-terra"),
        ("luna", "codex", "gpt-5.6-luna"),
        ("gemini", "gemini", "gemini-3.7-flash"),
        ("grok", "grok", "grok-4.6"),
    ] {
        let assignment = &crews.get(name).expect("built-in crew").assignment;
        assert_eq!(assignment.provider, provider);
        assert_eq!(assignment.model, model);
    }
    assert!(!crews.contains_key("claude"));
    assert!(!crews.contains_key("codex"));

    let config = RuntimeConfig::default_for_data_root(Path::new(".orbit"));
    assert_eq!(config.default_crew.as_deref(), Some("opus"));
}

fn load_config(body: &str) -> Result<RuntimeConfig, OrbitError> {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), body);
    RuntimeConfig::load_layered(global.path(), workspace.path())
}

#[test]
fn crew_description_and_tags_normalize_without_loss() {
    let config = load_config(
        r#"
[workflow]
default_crew = "sol"

[crews.sol]
model = "gpt-test"
provider = "codex"
backend = "cli"
description = "  Systems implementation  "
tags = [" review ", "", "hard", "review"]
"#,
    )
    .expect("config loads");
    let crew = config.crews.get("sol").expect("sol crew");
    assert_eq!(crew.description.as_deref(), Some("Systems implementation"));
    assert_eq!(crew.tags, vec!["hard", "review"]);
}

#[test]
fn retired_duel_config_written_by_orbit_init_loads_and_is_ignored() {
    let config = load_config(
        r#"
[workflow]
base_branch = "agent-main"

[duel]
candidates = ["claude", "codex", "gemini"]

[duel.models]
claude = "opus"
codex = "gpt-5.6-sol"
gemini = "pro"
"#,
    )
    .expect("retired init-era duel config must load");

    assert_eq!(config.workflow_base_branch(), "agent-main");
    assert!(config.snapshot.value_for("duel.candidates").is_none());
    assert!(config.snapshot.value_for("duel.models").is_none());
    assert!(RETIRED_DUEL_CONFIG_WARNING.contains("[duel]"));
    assert!(RETIRED_DUEL_CONFIG_WARNING.contains("[duel.models]"));
}

#[test]
fn deprecated_task_id_pattern_loads_valid_regex_from_workspace_config() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        "[knowledge]\ntask_id_pattern = \"[A-Z]+-\\\\d+\"\n",
    );

    RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
}

#[test]
fn deprecated_task_id_pattern_ignores_invalid_regex_at_load_time() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        "[knowledge]\ntask_id_pattern = \"[unclosed\"\n",
    );

    RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("deprecated invalid regex must load");
}

#[test]
fn deprecated_task_id_pattern_ignores_empty_string() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[knowledge]\ntask_id_pattern = \"  \"\n");

    RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("deprecated empty pattern must load");
}

#[test]
fn deprecated_task_id_pattern_absent_when_section_absent() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[scoring]\nenabled = true\n");

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert_eq!(config.pr_config().task_url_template.as_deref(), None);
}

#[test]
fn pr_config_defaults_to_no_task_url_template_without_config() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");

    assert_eq!(config.pr_config().task_url_template.as_deref(), None);
}

#[test]
fn pr_task_url_template_loads_from_workspace_config() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        "[pr]\ntask_url_template = \"https://orbit-cli.com/tasks/{task_id}\"\n",
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");

    assert_eq!(
        config.pr_config().task_url_template.as_deref(),
        Some("https://orbit-cli.com/tasks/{task_id}")
    );
}

/// [ORB-10801] `[runtime] backend` selected the retired agent-loop execution
/// backend. `cli` named the surviving path, so it stays accepted and inert.
#[test]
fn retired_runtime_backend_cli_is_accepted_and_ignored() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[runtime]\nbackend = \"cli\"\n");

    RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("`backend = \"cli\"` must keep loading");
}

/// [ORB-10801] `ORBIT_BACKEND` was tier 2 of the same retired chain, and gets
/// the same treatment: `cli` is inert, the removed values fail closed.
#[test]
fn retired_backend_env_override_is_inert_for_cli_and_fails_closed_otherwise() {
    let empty = toml::Value::Table(toml::map::Map::new());

    for accepted in [None, Some(""), Some("cli")] {
        super::super::runtime::retired_backend_override_check(&empty, accepted)
            .unwrap_or_else(|error| panic!("{accepted:?} must be accepted: {error}"));
    }

    for removed in ["http", "auto"] {
        let error = super::super::runtime::retired_backend_override_check(&empty, Some(removed))
            .expect_err("a removed ORBIT_BACKEND value must fail closed");
        let message = error.to_string();
        assert!(message.contains("ORBIT_BACKEND"), "message: {message}");
        assert!(message.contains(removed), "message: {message}");
        assert!(message.contains("CLI agent path"), "message: {message}");
    }
}

/// [ORB-10801] The removed values fail closed rather than being reinterpreted
/// as CLI agent execution behind the operator's back.
#[test]
fn retired_runtime_backend_http_fails_closed_with_migration() {
    for removed in ["http", "auto", "clii"] {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            workspace.path(),
            &format!("[runtime]\nbackend = \"{removed}\"\n"),
        );

        let error = RuntimeConfig::load_layered(global.path(), workspace.path())
            .expect_err("retired backend value must fail config load");
        let message = error.to_string();

        assert!(message.contains("[runtime]"), "message: {message}");
        assert!(message.contains(removed), "message: {message}");
        assert!(message.contains("CLI agent path"), "message: {message}");
    }
}

#[test]
fn crews_load_when_present_and_well_formed() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.codex]
model = "gpt-5.5"
provider = "codex"
backend = "cli"

[workflow]
default_crew = "codex"
"#,
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");

    assert_eq!(config.default_crew.as_deref(), Some("codex"));
    assert_eq!(
        config
            .crews
            .get("codex")
            .expect("crew exists")
            .assignment
            .model,
        orbit_common::test_fixtures::TEST_CODEX_MODEL
    );
}

#[test]
fn absent_crews_use_built_in_defaults() {
    let config = load_config("[scoring]\nenabled = true\n").expect("config without crews loads");

    assert_eq!(config.crews, default_crews());
    assert_eq!(config.default_crew.as_deref(), Some("opus"));
}

#[test]
fn explicitly_empty_crews_preserve_an_empty_registry() {
    let config = load_config("[crews]\n").expect("empty crew registry loads");

    assert!(config.crews.is_empty());
    assert_eq!(config.default_crew, None);
}

#[test]
fn default_crew_must_reference_defined_crew() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.codex]
model = "gpt-5.5"
provider = "codex"
backend = "cli"

[workflow]
default_crew = "missing"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("unknown default crew fails");

    assert!(matches!(error, OrbitError::InvalidInputDiagnostic { .. }));
    assert_eq!(error.did_you_mean(), Some(&["codex".to_string()][..]));
}

#[test]
fn default_crew_unset_with_custom_crews_fails_load() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    // Only a non-seeded crew defined; no [workflow] table at all.
    write_config(
        workspace.path(),
        r#"
[crews.my-team]
model = "gpt-5.5"
provider = "codex"
backend = "cli"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("missing default_crew with non-seeded crews must fail");

    let message = error.to_string();
    assert!(matches!(error, OrbitError::InvalidInput(_)), "{message}");
    assert!(message.contains("[workflow].default_crew"), "{message}");
    assert!(message.contains("my-team"), "{message}");
}

#[test]
fn default_crew_unset_with_seeded_crew_still_loads() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    // The canonical claude system crew is present, so the fallback applies.
    write_config(
        workspace.path(),
        r#"
[crews.claude]
model = "opus"
provider = "claude"
backend = "cli"
"#,
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert_eq!(config.default_crew.as_deref(), Some("claude"));
}

#[test]
fn workflow_default_crew_no_crews_defined_is_noop() {
    use super::super::registry::resolve_default_crew;
    let crews = BTreeMap::new();

    let default_crew = resolve_default_crew(None, &crews, None).expect("empty registry is allowed");

    assert_eq!(default_crew, None);
}

#[test]
fn workflow_default_crew_uses_environment_then_claude_system_default() {
    use super::super::registry::resolve_default_crew;
    let crews = BTreeMap::from([
        ("opus".to_string(), single_family_crew("claude")),
        ("sol".to_string(), single_family_crew("codex")),
        ("gemini".to_string(), single_family_crew("gemini")),
    ]);

    let env = resolve_default_crew(None, &crews, Some("google"))
        .expect("deprecated environment alias resolves");
    assert_eq!(env.as_deref(), Some("gemini"));

    let codex = resolve_default_crew(None, &crews, Some("codex"))
        .expect("provider environment maps to standard crew");
    assert_eq!(codex.as_deref(), Some("sol"));

    let system =
        resolve_default_crew(None, &crews, None).expect("canonical system default resolves");
    assert_eq!(system.as_deref(), Some("opus"));

    let error = resolve_default_crew(None, &crews, Some("bogus"))
        .expect_err("selected invalid environment value must not fall back");
    assert!(error.to_string().contains("CONSTELLATION_DEFAULT_PROVIDER"));
}

/// [ORB-10801] `[crews.<name>] backend` pinned the same retired selector. A
/// crew that still declares `cli` keeps loading; the removed values are
/// refused rather than re-pointed at the CLI agent silently.
#[test]
fn retired_crew_backend_is_inert_for_cli_and_fails_closed_otherwise() {
    load_config(
        r#"
[crews.legacy]
model = "gpt-test"
provider = "codex"
backend = "cli"

[workflow]
default_crew = "legacy"
"#,
    )
    .expect("`backend = \"cli\"` must keep loading");

    for removed in ["http", "auto"] {
        let error = load_config(&format!(
            r#"
[crews.legacy]
model = "gpt-test"
provider = "codex"
backend = "{removed}"

[workflow]
default_crew = "legacy"
"#
        ))
        .expect_err("a removed crew backend must fail config load");
        let message = error.to_string();
        assert!(message.contains("[crews.legacy]"), "message: {message}");
        assert!(message.contains(removed), "message: {message}");
        assert!(message.contains("CLI agent path"), "message: {message}");
    }
}

#[test]
fn flat_crews_with_incomplete_assignment_fail_load() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.codex]
provider = "codex"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("incomplete crew fails");

    assert!(matches!(error, OrbitError::InvalidInput(_)));
    assert!(error.to_string().contains("[crews.codex]"));
    assert!(error.to_string().contains("model"));
}

#[test]
fn legacy_role_tables_fail_with_flat_shape_rewrite_guidance() {
    let error = load_config(
        r#"
[crews.legacy]
planner = { model = "planner-model", provider = "claude", backend = "cli" }
implementer = { model = "implementer-model", provider = "codex", backend = "cli" }
reviewer = { model = "reviewer-model", provider = "gemini", backend = "cli" }

[workflow]
default_crew = "legacy"
"#,
    )
    .expect_err("legacy role tables must fail config load");
    let message = error.to_string();

    for expected in [
        "[crews.legacy]",
        "planner/implementer/reviewer",
        "model",
        "provider",
    ] {
        assert!(
            message.contains(expected),
            "expected {message:?} to contain {expected:?}"
        );
    }
}

#[test]
fn flat_crew_mixed_with_role_tables_fails_with_flat_shape_rewrite_guidance() {
    let error = load_config(
        r#"
[crews.mixed]
model = "gpt-test"
provider = "codex"
backend = "cli"
implementer = { model = "gpt-test", provider = "codex", backend = "cli" }
"#,
    )
    .expect_err("mixed crew shape must fail config load");
    let message = error.to_string();

    for expected in [
        "[crews.mixed]",
        "planner/implementer/reviewer",
        "model",
        "provider",
    ] {
        assert!(
            message.contains(expected),
            "expected {message:?} to contain {expected:?}"
        );
    }
}

#[test]
fn task_artifact_store_rejects_removed_key() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[task]\nartifact_store = \"v2\"\n");

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("artifact store selector must be rejected");
    let message = error.to_string();

    assert!(message.contains("[task] artifact_store"));
    assert!(message.contains("no longer supported"));
    assert!(message.contains("v2"));
}

#[test]
fn workflow_auto_ship_defaults_false_and_loads_when_set() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");

    write_config(workspace.path(), "");
    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert!(!config.workflow_auto_ship());

    write_config(
        workspace.path(),
        r#"
[workflow]
auto_ship = true
"#,
    );
    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert!(config.workflow_auto_ship());
}

#[test]
fn workspace_single_key_inherits_other_global_keys_then_built_in_defaults() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        global.path(),
        "[workflow]\nbase_branch = \"global-branch\"\n[scoring]\nenabled = false\n",
    );
    write_config(workspace.path(), "[workflow]\nauto_ship = true\n");

    let config = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("workspace config loads");

    assert!(config.workflow_auto_ship());
    assert_eq!(config.workflow_base_branch(), "global-branch");
    assert!(!config.scoring_enabled);
}

#[test]
fn workspace_file_does_not_inherit_security_relevant_global_keys() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        global.path(),
        r#"
[execution.codex]
sandbox = "danger-full-access"
approval_policy = "on-request"

[execution.env]
inherit = true
pass = ["GLOBAL_SECRET"]
"#,
    );
    write_config(workspace.path(), "[scoring]\nenabled = false\n");

    let config = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("workspace config loads");

    assert_eq!(config.codex_execution.sandbox(), "workspace-write");
    assert_eq!(config.codex_execution.approval_policy(), None);
    assert_eq!(
        config.snapshot.execution_env_pass,
        ConfigSnapshot::default().execution_env_pass
    );
    assert!(!config.execution_env.inherit());
}

#[test]
fn workspace_crew_field_override_keeps_global_crew_fields_and_other_crews() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        global.path(),
        r#"
[workflow]
default_crew = "build"

[crews.build]
model = "global-model"
provider = "codex"
backend = "cli"

[crews.review]
model = "review-model"
provider = "claude"
backend = "cli"
"#,
    );
    write_config(
        workspace.path(),
        r#"
[crews.build]
model = "workspace-model"
"#,
    );

    let config = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("layered crew config loads");

    let build = config.crews.get("build").expect("overridden crew remains");
    assert_eq!(build.assignment.model, "workspace-model");
    assert_eq!(build.assignment.provider, "codex");
    assert_eq!(
        config
            .crews
            .get("review")
            .expect("global-only crew remains")
            .assignment
            .model,
        "review-model"
    );
}

#[test]
fn layered_config_rejects_legacy_global_crew_before_flat_workspace_override() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        global.path(),
        r#"
[workflow]
default_crew = "build"

[crews.build]
planner = { model = "old-model", provider = "codex", backend = "cli" }
implementer = { model = "old-model", provider = "codex", backend = "cli" }
reviewer = { model = "old-model", provider = "codex", backend = "cli" }
"#,
    );
    write_config(
        workspace.path(),
        r#"
[crews.build]
model = "new-model"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("legacy global crew must not be masked by workspace fields");
    let message = error.to_string();

    assert!(message.contains("[crews.build]"), "{message}");
    assert!(
        message.contains("planner/implementer/reviewer"),
        "{message}"
    );
}

#[test]
fn effective_config_attributes_values_to_workspace_global_and_built_in_sources() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        global.path(),
        r#"
[workflow]
base_branch = "integration"
default_crew = "build"

[execution.codex]
sandbox = "danger-full-access"

[crews.build]
model = "global-model"
provider = "codex"
backend = "cli"
"#,
    );
    write_config(
        workspace.path(),
        r#"
[scoring]
enabled = false

[crews.build]
model = "workspace-model"
"#,
    );

    let effective =
        load_effective_config(global.path(), workspace.path()).expect("effective config loads");
    let values = effective
        .values()
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        values["scoring.enabled"].source.kind(),
        ConfigValueSourceKind::Workspace
    );
    assert_eq!(
        values["workflow.base_branch"].source.kind(),
        ConfigValueSourceKind::Global
    );
    assert_eq!(
        values["execution.codex.sandbox"].source.kind(),
        ConfigValueSourceKind::BuiltIn
    );
    assert_eq!(
        values["crews.build.model"].source.kind(),
        ConfigValueSourceKind::Workspace
    );
    assert_eq!(
        values["crews.build.provider"].source.kind(),
        ConfigValueSourceKind::Global
    );
    let workspace_config_path = workspace.path().join("config.toml");
    assert_eq!(
        values["scoring.enabled"].source.path(),
        Some(workspace_config_path.as_path())
    );
    let global_config_path = global.path().join("config.toml");
    assert_eq!(
        values["crews.build.model"].source.path(),
        Some(workspace_config_path.as_path())
    );
    assert_eq!(
        values["crews.build.provider"].source.path(),
        Some(global_config_path.as_path())
    );
}

#[test]
fn module_and_user_docs_share_the_layering_contract() {
    const CONTRACT: &str = "Ordinary settings inherit per key: workspace values override global values, global values fill omissions, and built-in defaults fill remaining gaps.";
    let module_docs = include_str!("../mod.rs");
    let user_docs = include_str!("../../../../../docs/CONFIG.md");

    assert!(module_docs.contains(CONTRACT));
    assert!(user_docs.contains(CONTRACT));
}

#[test]
fn shipped_default_config_has_no_workspace_specific_identifiers() {
    let shipped = include_str!("../../../assets/config/default-config.toml");

    for forbidden in ["ORB-", "agent-main", "dk-server", "F2026-"] {
        assert!(
            !shipped.contains(forbidden),
            "shipped config must not contain workspace-specific marker {forbidden:?}"
        );
    }
}

#[test]
fn runtime_log_rotation_rejects_invalid_values() {
    // [ORB-00415] Malformed rotation knobs must fail at config load with a
    // clear, key-naming error.
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");

    write_config(workspace.path(), "[runtime]\nlog_retention_days = 0\n");
    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("zero retention must fail config load");
    assert!(
        error.to_string().contains("log_retention_days"),
        "message: {error}"
    );

    write_config(
        workspace.path(),
        "[runtime]\nlog_max_total_mb = 10\nlog_max_file_mb = 50\n",
    );
    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("per-file budget above total must fail config load");
    assert!(
        error.to_string().contains("log_max_file_mb"),
        "message: {error}"
    );
}

#[test]
fn runtime_log_rotation_accepts_valid_values() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        "[runtime]\nlog_retention_days = 14\nlog_max_total_mb = 200\nlog_max_file_mb = 20\n",
    );
    RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect("valid log rotation config should load");
}

use super::super::runtime::*;
use orbit_common::types::{Crew, CrewRoleAssignment, OrbitError, all_agent_families};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::tempdir;

fn write_config(dir: &Path, body: &str) {
    std::fs::write(dir.join("config.toml"), body).expect("write config");
}

fn single_family_crew(name: &str) -> Crew {
    let role = CrewRoleAssignment {
        model: format!("{name}-model"),
        provider: name.to_string(),
        backend: "cli".to_string(),
    };
    Crew {
        name: name.to_string(),
        assignment: role,
        description: None,
        tags: Vec::new(),
    }
}

fn load_config(body: &str) -> Result<RuntimeConfig, OrbitError> {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), body);
    RuntimeConfig::load_layered(global.path(), workspace.path())
}

fn assert_invalid_duel_config(body: &str, substrings: &[&str]) {
    let error = load_config(body).expect_err("invalid duel config must fail");
    let message = error.to_string();
    assert!(matches!(error, OrbitError::InvalidInput(_)), "{message}");
    for substring in substrings {
        assert!(
            message.contains(substring),
            "expected {message:?} to contain {substring:?}"
        );
    }
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
fn duel_config_loads_candidates_and_models() {
    let config = load_config(
        r#"
[duel]
candidates = [" Codex ", "CLAUDE", "gemini"]

[duel.models]
" Codex " = " gpt-5.5 "
CLAUDE = " opus-4.7 "
"#,
    )
    .expect("config loads");

    let mut expected_models = BTreeMap::new();
    expected_models.insert("claude".to_string(), "opus-4.7".to_string());
    expected_models.insert(
        "codex".to_string(),
        orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string(),
    );
    assert_eq!(
        config.duel,
        DuelConfig {
            candidates: vec![
                "codex".to_string(),
                "claude".to_string(),
                "gemini".to_string()
            ],
            models: expected_models,
        }
    );
}

#[test]
fn duel_config_defaults_to_all_families_without_section() {
    let config = load_config("[scoring]\nenabled = true\n").expect("config loads");

    assert_eq!(
        config.duel.candidates,
        all_agent_families()
            .iter()
            .map(|family| (*family).to_string())
            .collect::<Vec<_>>()
    );
    assert!(config.duel.models.is_empty());
}

#[test]
fn duel_config_rejects_empty_candidates() {
    assert_invalid_duel_config(
        "[duel]\ncandidates = []\n",
        &["candidates", "at least 3", "codex, claude, gemini, grok"],
    );
}

#[test]
fn duel_config_rejects_fewer_than_three_distinct_candidates() {
    assert_invalid_duel_config(
        "[duel]\ncandidates = [\"codex\", \"claude\"]\n",
        &["3 distinct", "codex, claude", "codex, claude, gemini, grok"],
    );
}

#[test]
fn duel_config_rejects_duplicate_candidates_after_normalization() {
    assert_invalid_duel_config(
        "[duel]\ncandidates = [\"codex\", \" Codex \", \"claude\"]\n",
        &["duplicate", "codex", "codex, claude, gemini, grok"],
    );
}

#[test]
fn duel_config_rejects_unknown_candidate() {
    assert_invalid_duel_config(
        "[duel]\ncandidates = [\"codex\", \"claude\", \"notabot\"]\n",
        &["notabot", "valid candidates", "codex, claude, gemini, grok"],
    );
}

#[test]
fn duel_config_rejects_model_key_outside_resolved_candidates() {
    assert_invalid_duel_config(
        r#"
[duel]
candidates = ["codex", "claude", "gemini"]

[duel.models]
grok = "grok-4"
"#,
        &[
            "grok",
            "resolved [duel].candidates",
            "codex, claude, gemini",
        ],
    );
}

#[test]
fn duel_config_rejects_empty_model_value() {
    assert_invalid_duel_config(
        r#"
[duel]
candidates = ["codex", "claude", "gemini"]

[duel.models]
codex = "   "
"#,
        &["duel.models", "codex", "   "],
    );
}

#[test]
fn deprecated_task_id_pattern_loads_valid_regex_from_workspace_config() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        "[knowledge]\ntask_id_pattern = \"[A-Z]+-\\\\d+\"\n",
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert!(config.v2_backend().is_none());
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
    assert!(config.v2_backend().is_none());
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

#[test]
fn runtime_backend_loads_auto_from_workspace_config() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[runtime]\nbackend = \"auto\"\n");

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");

    assert_eq!(config.v2_backend(), Some("auto"));
}

#[test]
fn runtime_backend_rejects_invalid_value() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(workspace.path(), "[runtime]\nbackend = \"clii\"\n");

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("invalid backend must fail config load");
    let message = error.to_string();

    assert!(message.contains("[runtime] backend"));
    assert!(message.contains("clii"));
    assert!(message.contains("http, cli, auto"));
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
        ("claude".to_string(), single_family_crew("claude")),
        ("gemini".to_string(), single_family_crew("gemini")),
    ]);

    let env = resolve_default_crew(None, &crews, Some("google"))
        .expect("deprecated environment alias resolves");
    assert_eq!(env.as_deref(), Some("gemini"));

    let system =
        resolve_default_crew(None, &crews, None).expect("canonical system default resolves");
    assert_eq!(system.as_deref(), Some("claude"));

    let error = resolve_default_crew(None, &crews, Some("bogus"))
        .expect_err("selected invalid environment value must not fall back");
    assert!(error.to_string().contains("CONSTELLATION_DEFAULT_PROVIDER"));
}

#[test]
fn flat_crews_with_incomplete_assignment_fail_load() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.codex]
model = "gpt-5.5"
provider = "codex"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("incomplete crew fails");

    assert!(matches!(error, OrbitError::InvalidInput(_)));
    assert!(error.to_string().contains("[crews.codex]"));
    assert!(error.to_string().contains("backend"));
}

#[test]
fn legacy_divergent_crew_uses_implementer_assignment() {
    let config = load_config(
        r#"
[crews.legacy]
planner = { model = "planner-model", provider = "claude", backend = "cli" }
implementer = { model = "implementer-model", provider = "codex", backend = "cli" }
reviewer = { model = "reviewer-model", provider = "gemini", backend = "cli" }

[workflow]
default_crew = "legacy"
"#,
    )
    .expect("legacy crew loads");

    let crew = config.crews.get("legacy").expect("legacy crew");
    assert_eq!(crew.assignment.model, "implementer-model");
    assert_eq!(crew.assignment.provider, "codex");
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
fn workspace_config_replaces_global_instead_of_merging() {
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
    assert_eq!(config.workflow_base_branch(), "main");
    assert!(config.scoring_enabled);
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

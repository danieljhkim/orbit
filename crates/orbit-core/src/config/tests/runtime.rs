use super::super::runtime::*;
use tempfile::tempdir;
use std::path::Path;
use orbit_common::types::{all_agent_families, OrbitError};
use std::collections::BTreeMap;

fn write_config(dir: &Path, body: &str) {
    std::fs::write(dir.join("config.toml"), body).expect("write config");
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
    expected_models.insert("codex".to_string(), "gpt-5.5".to_string());
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
[crews.opus-codex]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
reviewer = { model = "gpt-5.5", provider = "codex", backend = "cli" }

[workflow]
default_crew = "opus-codex"
"#,
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");

    assert_eq!(config.default_crew.as_deref(), Some("opus-codex"));
    assert_eq!(
        config
            .crews
            .get("opus-codex")
            .expect("crew exists")
            .implementer
            .model,
        "gpt-5.5"
    );
}

#[test]
fn default_crew_must_reference_defined_crew() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.opus-codex]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
reviewer = { model = "gpt-5.5", provider = "codex", backend = "cli" }

[workflow]
default_crew = "missing"
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("unknown default crew fails");

    assert!(matches!(error, OrbitError::InvalidInputDiagnostic { .. }));
    assert_eq!(error.did_you_mean(), Some(&["opus-codex".to_string()][..]));
}

#[test]
fn default_crew_unset_with_custom_crews_fails_load() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    // Only a non-"opus-codex" crew defined; no [workflow] table at all.
    write_config(
        workspace.path(),
        r#"
[crews.my-team]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
reviewer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
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
    // opus-codex is still present, so the historical fallback applies.
    write_config(
        workspace.path(),
        r#"
[crews.opus-codex]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
reviewer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
"#,
    );

    let config =
        RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
    assert_eq!(config.default_crew.as_deref(), Some("opus-codex"));
}

#[test]
fn crews_with_incomplete_role_fail_load() {
    let global = tempdir().expect("global tempdir");
    let workspace = tempdir().expect("workspace tempdir");
    write_config(
        workspace.path(),
        r#"
[crews.opus-codex]
planner = { model = "claude-opus-4-7", provider = "claude", backend = "cli" }
implementer = { model = "gpt-5.5", provider = "codex", backend = "cli" }
"#,
    );

    let error = RuntimeConfig::load_layered(global.path(), workspace.path())
        .expect_err("incomplete crew fails");

    assert!(matches!(error, OrbitError::InvalidInput(_)));
    assert!(error.to_string().contains("[crews.opus-codex]"));
    assert!(error.to_string().contains("reviewer"));
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

use super::super::agent_detect::{DetectedAgents, detect, testing::MockAgentEnvProbe};
use super::super::bootstrap::*;
use super::super::raw::RawAgentRoleConfig;
use super::super::raw::RawRuntimeConfig;
use super::super::runtime::RuntimeConfig;
use orbit_common::types::all_agent_families;
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_roles() -> BTreeMap<String, RawAgentRoleConfig> {
    let mut roles = BTreeMap::new();
    roles.insert(
        "reviewer".to_string(),
        RawAgentRoleConfig {
            provider: Some("claude".into()),
            backend: Some("cli".into()),
            model: Some("claude-opus-4-7".into()),
        },
    );
    roles.insert(
        "implementer".to_string(),
        RawAgentRoleConfig {
            provider: Some("codex".into()),
            backend: Some("cli".into()),
            model: Some("gpt-5.5".into()),
        },
    );
    roles.insert(
        "planner".to_string(),
        RawAgentRoleConfig {
            provider: Some("claude".into()),
            backend: Some("http".into()),
            model: Some("claude-opus-4-7".into()),
        },
    );
    roles
}

#[test]
fn default_template_keeps_agent_dependent_sections_out() {
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("default_crew"));
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("[crews."));
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("[duel"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[execution.env]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[execution.codex]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[task.approval]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[scoring]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[graph]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[workflow]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("base_branch = \"main\""));
}

#[test]
fn seed_with_claude_detection_writes_all_crews_and_claude_default() {
    let detected = detect(&MockAgentEnvProbe::new().with_binary("claude"));
    let contents = seed_contents(&detected, None);

    assert_all_base_crews_present(&contents);
    assert!(contents.contains("default_crew = \"claude\""));
    assert!(!contents.contains("[duel"));
}

#[test]
fn seed_with_empty_detection_defaults_codex_and_omits_duel() {
    let detected = DetectedAgents::default();
    let contents = seed_contents(&detected, None);

    assert_all_base_crews_present(&contents);
    assert!(contents.contains("default_crew = \"codex\""));
    assert!(!contents.contains("[duel"));
}

#[test]
fn seed_with_three_available_families_writes_duel_candidates_and_models() {
    let detected = detect(
        &MockAgentEnvProbe::new()
            .with_binary("claude")
            .with_binary("codex")
            .with_binary("gemini"),
    );
    let contents = seed_contents(&detected, None);
    let parsed: toml::Value = toml::from_str(&contents).expect("parse seeded config");

    let candidates = parsed
        .get("duel")
        .and_then(|duel| duel.get("candidates"))
        .and_then(|candidates| candidates.as_array())
        .expect("duel candidates");
    let candidates: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.as_str().expect("candidate string"))
        .collect();
    assert_eq!(candidates, vec!["claude", "codex", "gemini"]);

    let models = parsed
        .get("duel")
        .and_then(|duel| duel.get("models"))
        .and_then(|models| models.as_table())
        .expect("duel models");
    assert_eq!(models.len(), 3);
    assert_eq!(
        models.get("claude").and_then(|v| v.as_str()),
        Some("claude-opus-4-7")
    );
    assert_eq!(
        models.get("codex").and_then(|v| v.as_str()),
        Some("gpt-5.5")
    );
    assert_eq!(
        models.get("gemini").and_then(|v| v.as_str()),
        Some("gemini-3-pro")
    );
}

#[test]
fn seed_with_fewer_than_three_families_omits_duel_and_runtime_falls_back() {
    let detected = detect(&MockAgentEnvProbe::new().with_binary("claude"));
    let contents = seed_contents(&detected, None);

    assert!(!contents.contains("[duel"));
    let config = load_seeded_config(&contents);
    let expected: Vec<String> = all_agent_families()
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(config.duel.candidates, expected);
}

#[test]
fn seeded_configs_round_trip_for_detection_permutations() {
    let cases = [
        ("none", DetectedAgents::default()),
        (
            "one cli",
            detect(&MockAgentEnvProbe::new().with_binary("claude")),
        ),
        (
            "two clis",
            detect(
                &MockAgentEnvProbe::new()
                    .with_binary("claude")
                    .with_binary("codex"),
            ),
        ),
        (
            "three clis",
            detect(
                &MockAgentEnvProbe::new()
                    .with_binary("claude")
                    .with_binary("codex")
                    .with_binary("gemini"),
            ),
        ),
        (
            "four clis",
            detect(
                &MockAgentEnvProbe::new()
                    .with_binary("claude")
                    .with_binary("codex")
                    .with_binary("gemini")
                    .with_binary("grok"),
            ),
        ),
        (
            "api keys only",
            detect(
                &MockAgentEnvProbe::new()
                    .with_env("ANTHROPIC_API_KEY", "anthropic")
                    .with_env("OPENAI_API_KEY", "openai")
                    .with_env("GEMINI_API_KEY", "gemini"),
            ),
        ),
    ];

    for (name, detected) in cases {
        let contents = seed_contents(&detected, None);
        toml::from_str::<RawRuntimeConfig>(&contents)
            .unwrap_or_else(|err| panic!("{name} raw parse failed: {err}"));
        load_seeded_config(&contents);
    }
}

#[test]
fn seed_with_no_role_settings_writes_generated_agent_block() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, None).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");
    assert!(no_active_role_section(&contents));
    assert_all_base_crews_present(&contents);
    assert!(contents.contains("default_crew = \"codex\""));
}

fn seed_contents(
    detected: &DetectedAgents,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> String {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let created = seed_default_config(&path, detected, role_settings).expect("seed");
    assert!(created);
    std::fs::read_to_string(&path).expect("read")
}

fn load_seeded_config(contents: &str) -> RuntimeConfig {
    let dir = tempdir().expect("tempdir");
    std::fs::write(dir.path().join("config.toml"), contents).expect("write config");
    RuntimeConfig::load_layered(dir.path(), dir.path()).expect("runtime config loads")
}

fn assert_all_base_crews_present(contents: &str) {
    assert!(contents.contains("[crews.claude]"));
    assert!(contents.contains("[crews.codex]"));
    assert!(contents.contains("[crews.gemini]"));
    assert!(contents.contains("[crews.grok]"));
}

fn no_active_role_section(contents: &str) -> bool {
    contents
        .lines()
        .all(|line| !line.trim_start().starts_with("[agent."))
}

#[test]
fn seed_with_role_settings_writes_custom_crew() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let roles = sample_roles();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&roles)).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");

    assert!(no_active_role_section(&contents));
    assert!(contents.contains("default_crew = \"custom\""));
    assert_all_base_crews_present(&contents);
    assert!(contents.contains("[crews.custom]"));
    assert!(contents.contains("provider = \"claude\""));
    assert!(contents.contains("provider = \"codex\""));
    assert!(contents.contains("model = \"claude-opus-4-7\""));
    assert!(contents.contains("model = \"gpt-5.5\""));

    // Round-trips through toml::from_str (consumer side will need this).
    let parsed: toml::Value = toml::from_str(&contents).expect("parse");
    let crews = parsed
        .get("crews")
        .expect("crews table")
        .as_table()
        .unwrap();
    let custom = crews
        .get("custom")
        .and_then(|v| v.as_table())
        .expect("custom crew");
    assert!(custom.contains_key("reviewer"));
    assert!(custom.contains_key("implementer"));
    assert!(custom.contains_key("planner"));
}

#[test]
fn seed_with_existing_file_is_noop() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# pre-existing user content\n").expect("preseed");

    let roles = sample_roles();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&roles)).expect("seed");
    assert!(!created);

    let contents = std::fs::read_to_string(&path).expect("read");
    assert_eq!(contents, "# pre-existing user content\n");
}

#[test]
fn seed_with_empty_role_map_uses_detected_default() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let roles: BTreeMap<String, RawAgentRoleConfig> = BTreeMap::new();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&roles)).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");
    assert!(contents.contains("default_crew = \"codex\""));
    assert_all_base_crews_present(&contents);
}

#[test]
fn seed_with_incomplete_role_settings_fails() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let mut roles = sample_roles();
    roles.get_mut("planner").expect("planner").model.take();
    let detected = DetectedAgents::default();
    let error =
        seed_default_config(&path, &detected, Some(&roles)).expect_err("missing model fails");
    assert!(
        error
            .to_string()
            .contains("custom crew role `planner` is missing required `model`")
    );
    assert!(!path.exists());
}

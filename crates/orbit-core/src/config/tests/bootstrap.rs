use super::super::agent_detect::{DetectedAgents, detect, testing::MockAgentEnvProbe};
use super::super::bootstrap::*;
use super::super::raw::RawCrewAssignment;
use super::super::raw::RawRuntimeConfig;
use super::super::runtime::RuntimeConfig;
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_crew_settings() -> BTreeMap<String, RawCrewAssignment> {
    BTreeMap::from([(
        "custom".to_string(),
        RawCrewAssignment {
            provider: Some("codex".into()),
            backend: Some("cli".into()),
            model: Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.into()),
        },
    )])
}

#[test]
fn default_template_keeps_agent_dependent_sections_out() {
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("default_crew"));
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("[crews."));
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("[duel"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[execution.env]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[execution.codex]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[scoring]"));
    assert!(!DEFAULT_CONFIG_TEMPLATE.contains("[graph]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("[workflow]"));
    assert!(DEFAULT_CONFIG_TEMPLATE.contains("base_branch = \"main\""));
}

#[test]
fn claude_only_seeds_the_claude_family_and_qa() {
    let contents = seed_contents(
        &detect(&MockAgentEnvProbe::new().with_binary("claude")),
        None,
    );
    let parsed = parsed_config(&contents);

    assert_eq!(crew_names(&parsed), vec!["fable", "opus", "qa", "sonnet"]);
    assert_crew(&parsed, "opus", "claude", "opus");
    assert_crew(&parsed, "sonnet", "claude", "sonnet");
    assert_crew(&parsed, "fable", "claude", "fable");
    assert_crew(&parsed, "qa", "claude", "sonnet");
    assert_default_crew(&parsed, Some("opus"));
    assert!(!contents.contains("[duel"));
}

#[test]
fn codex_only_seeds_the_codex_family_and_qa() {
    let contents = seed_contents(
        &detect(&MockAgentEnvProbe::new().with_binary("codex")),
        None,
    );
    let parsed = parsed_config(&contents);

    assert_eq!(crew_names(&parsed), vec!["luna", "qa", "sol", "terra"]);
    assert_crew(&parsed, "sol", "codex", "gpt-5.6-sol");
    assert_crew(&parsed, "terra", "codex", "gpt-5.6-terra");
    assert_crew(&parsed, "luna", "codex", "gpt-5.6-luna");
    assert_crew(&parsed, "qa", "codex", "gpt-5.6-terra");
    assert_default_crew(&parsed, Some("sol"));
}

#[test]
fn gemini_only_seeds_gemini_without_qa() {
    let contents = seed_contents(
        &detect(&MockAgentEnvProbe::new().with_binary("gemini")),
        None,
    );
    let parsed = parsed_config(&contents);

    assert_eq!(crew_names(&parsed), vec!["gemini"]);
    assert_crew(&parsed, "gemini", "gemini", "pro");
    assert_default_crew(&parsed, Some("gemini"));
}

#[test]
fn grok_only_seeds_grok_without_qa() {
    let contents = seed_contents(&detect(&MockAgentEnvProbe::new().with_binary("grok")), None);
    let parsed = parsed_config(&contents);

    assert_eq!(crew_names(&parsed), vec!["grok"]);
    assert_crew(&parsed, "grok", "grok", "grok-4.6");
    assert_default_crew(&parsed, Some("grok"));
}

#[test]
fn no_supported_cli_seeds_no_crews_or_dangling_default() {
    let detected = detect(
        &MockAgentEnvProbe::new()
            .with_binary("ollama")
            .with_env("ANTHROPIC_API_KEY", "anthropic")
            .with_env("OPENAI_API_KEY", "openai")
            .with_env("GEMINI_API_KEY", "gemini"),
    );
    let contents = seed_contents(&detected, None);
    let parsed = parsed_config(&contents);

    assert!(crew_names(&parsed).is_empty());
    assert_default_crew(&parsed, None);
    assert!(!contents.contains("[duel"));
    toml::from_str::<RawRuntimeConfig>(&contents).expect("no-provider config parses");
    let runtime = load_seeded_config(&contents);
    assert!(runtime.crews.is_empty());
    assert_eq!(runtime.default_crew, None);
}

#[test]
fn multi_provider_seed_includes_each_available_family_and_excludes_unavailable() {
    let detected = detect(
        &MockAgentEnvProbe::new()
            .with_binary("claude")
            .with_binary("codex")
            .with_binary("grok"),
    );
    let parsed = parsed_config(&seed_contents(&detected, None));

    assert_eq!(
        crew_names(&parsed),
        vec![
            "fable", "grok", "luna", "opus", "qa", "sol", "sonnet", "terra"
        ]
    );
    assert_default_crew(&parsed, Some("opus"));
    assert_crew(&parsed, "opus", "claude", "opus");
    assert_crew(&parsed, "sonnet", "claude", "sonnet");
    assert_crew(&parsed, "fable", "claude", "fable");
    assert_crew(&parsed, "sol", "codex", "gpt-5.6-sol");
    assert_crew(&parsed, "terra", "codex", "gpt-5.6-terra");
    assert_crew(&parsed, "luna", "codex", "gpt-5.6-luna");
    assert_crew(&parsed, "grok", "grok", "grok-4.6");
    assert_crew(&parsed, "qa", "codex", "gpt-5.6-terra");
    for crew in crews(&parsed).values() {
        assert_eq!(
            crew.get("backend").and_then(toml::Value::as_str),
            Some("cli")
        );
        assert_ne!(
            crew.get("provider").and_then(toml::Value::as_str),
            Some("gemini")
        );
    }
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
fn seed_with_no_crew_settings_keeps_static_template_content() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, None).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");
    assert!(no_active_role_section(&contents));
    assert!(crew_names(&parsed_config(&contents)).is_empty());
    assert!(!contents.contains("default_crew"));
    assert!(contents.contains("sandbox = \"danger-full-access\""));
}

fn seed_contents(
    detected: &DetectedAgents,
    crew_settings: Option<&BTreeMap<String, RawCrewAssignment>>,
) -> String {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let created = seed_default_config(&path, detected, crew_settings).expect("seed");
    assert!(created);
    std::fs::read_to_string(&path).expect("read")
}

fn load_seeded_config(contents: &str) -> RuntimeConfig {
    let dir = tempdir().expect("tempdir");
    std::fs::write(dir.path().join("config.toml"), contents).expect("write config");
    RuntimeConfig::load_layered(dir.path(), dir.path()).expect("runtime config loads")
}

fn parsed_config(contents: &str) -> toml::Value {
    toml::from_str(contents).expect("parse seeded config")
}

fn crews(parsed: &toml::Value) -> &toml::map::Map<String, toml::Value> {
    parsed
        .get("crews")
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| empty_toml_table())
}

fn empty_toml_table() -> &'static toml::map::Map<String, toml::Value> {
    static EMPTY: std::sync::OnceLock<toml::map::Map<String, toml::Value>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(toml::map::Map::new)
}

fn crew_names(parsed: &toml::Value) -> Vec<&str> {
    crews(parsed).keys().map(String::as_str).collect()
}

fn assert_crew(parsed: &toml::Value, name: &str, provider: &str, model: &str) {
    let crew = crews(parsed).get(name).expect("expected crew");
    assert_eq!(
        crew.get("provider").and_then(toml::Value::as_str),
        Some(provider)
    );
    assert_eq!(crew.get("model").and_then(toml::Value::as_str), Some(model));
    assert_eq!(
        crew.get("backend").and_then(toml::Value::as_str),
        Some("cli")
    );
}

fn assert_default_crew(parsed: &toml::Value, expected: Option<&str>) {
    assert_eq!(
        parsed
            .get("workflow")
            .and_then(|workflow| workflow.get("default_crew"))
            .and_then(toml::Value::as_str),
        expected,
    );
}

fn no_active_role_section(contents: &str) -> bool {
    contents
        .lines()
        .all(|line| !line.trim_start().starts_with("[agent."))
}

#[test]
fn seed_with_crew_settings_writes_custom_crew() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let settings = sample_crew_settings();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&settings)).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");

    assert!(no_active_role_section(&contents));
    assert!(contents.contains("default_crew = \"custom\""));
    assert!(contents.contains("[crews.custom]"));
    assert!(contents.contains("provider = \"codex\""));
    assert!(contents.contains(&format!(
        "model = \"{}\"",
        orbit_common::test_fixtures::TEST_CODEX_MODEL
    )));

    // Round-trips through toml::from_str (consumer side will need this).
    let parsed: toml::Value = toml::from_str(&contents).expect("parse");
    let crews = parsed
        .get("crews")
        .expect("crews table")
        .as_table()
        .unwrap();
    assert_eq!(crews.len(), 1, "custom init must not invent provider crews");
    let custom = crews
        .get("custom")
        .and_then(|v| v.as_table())
        .expect("custom crew");
    assert_eq!(
        custom.get("provider").and_then(|v| v.as_str()),
        Some("codex")
    );
    assert_eq!(custom.get("backend").and_then(|v| v.as_str()), Some("cli"));
    assert_eq!(
        custom.get("model").and_then(|v| v.as_str()),
        Some(orbit_common::test_fixtures::TEST_CODEX_MODEL)
    );
}

#[test]
fn seed_with_existing_file_is_noop() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# pre-existing user content\n").expect("preseed");

    let settings = sample_crew_settings();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&settings)).expect("seed");
    assert!(!created);

    let contents = std::fs::read_to_string(&path).expect("read");
    assert_eq!(contents, "# pre-existing user content\n");
}

#[test]
fn seed_with_empty_crew_map_uses_no_provider_behavior() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let settings: BTreeMap<String, RawCrewAssignment> = BTreeMap::new();
    let detected = DetectedAgents::default();
    let created = seed_default_config(&path, &detected, Some(&settings)).expect("seed");
    assert!(created);
    let contents = std::fs::read_to_string(&path).expect("read");
    let parsed = parsed_config(&contents);
    assert!(crew_names(&parsed).is_empty());
    assert_default_crew(&parsed, None);
}

#[test]
fn seed_with_incomplete_crew_settings_fails() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let mut settings = sample_crew_settings();
    settings.get_mut("custom").expect("custom").model.take();
    let detected = DetectedAgents::default();
    let error =
        seed_default_config(&path, &detected, Some(&settings)).expect_err("missing model fails");
    assert!(
        error
            .to_string()
            .contains("custom crew is missing required `model`")
    );
    assert!(!path.exists());
}

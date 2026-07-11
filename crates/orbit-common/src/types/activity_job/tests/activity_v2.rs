use super::super::activity_v2::*;
use serde::Deserialize;

/// Pinned local copy of the Constellation provider-resolution contract
/// (ORB-10091). The table-driven tests below assert the orbit-common `Provider`
/// surface against every row so canonical ids, aliases, and capability
/// predicates cannot drift from the shared contract.
const CONTRACT_JSON: &str = include_str!("fixtures/provider_contract.json");

#[derive(Debug, Deserialize)]
struct ProviderContract {
    contract_version: String,
    providers: Vec<ProviderRow>,
    unknown_provider_examples: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderRow {
    canonical: String,
    aliases: Vec<String>,
    has_cli_runtime: bool,
    has_http_transport: bool,
    worker_executable: bool,
}

fn load_contract() -> ProviderContract {
    serde_json::from_str(CONTRACT_JSON).expect("parse pinned provider contract fixture")
}

#[test]
fn provider_contract_fixture_is_pinned() {
    let contract = load_contract();
    // Pin the version so an upstream contract change forces a conscious bump.
    assert_eq!(contract.contract_version, "1.0.0");
    // Every canonical variant must appear exactly once; adding a `Provider`
    // variant without a fixture row (or vice-versa) fails here.
    assert_eq!(contract.providers.len(), Provider::ALL.len());
}

#[test]
fn provider_surface_matches_contract_rows() {
    let contract = load_contract();
    for row in &contract.providers {
        let provider = Provider::parse(&row.canonical)
            .unwrap_or_else(|_| panic!("canonical id '{}' must parse", row.canonical));
        assert_eq!(provider.as_str(), row.canonical, "as_str round-trip");
        assert_eq!(provider.to_string(), row.canonical, "Display round-trip");
        assert_eq!(
            provider.has_cli_runtime(),
            row.has_cli_runtime,
            "has_cli_runtime for {}",
            row.canonical
        );
        assert_eq!(
            provider.has_http_transport(),
            row.has_http_transport,
            "has_http_transport for {}",
            row.canonical
        );
        assert_eq!(
            provider.is_worker_executable(),
            row.worker_executable,
            "is_worker_executable for {}",
            row.canonical
        );
        for alias in &row.aliases {
            assert_eq!(
                Provider::parse(alias).expect("alias must parse"),
                provider,
                "alias '{alias}' resolves to {}",
                row.canonical
            );
        }
    }
}

#[test]
fn provider_parse_normalizes_case_and_whitespace() {
    for (raw, expected) in [
        ("  claude ", Provider::Claude),
        ("CLAUDE", Provider::Claude),
        ("Codex", Provider::Codex),
        ("OPENAI_COMPAT", Provider::OpenaiCompat),
        ("openai-compat", Provider::OpenaiCompat),
        (" OpenAI-Compat ", Provider::OpenaiCompat),
    ] {
        assert_eq!(Provider::parse(raw).expect("parse"), expected, "raw={raw}");
    }
}

#[test]
fn provider_parse_rejects_unknown_without_fallback() {
    let contract = load_contract();
    for raw in &contract.unknown_provider_examples {
        let err = Provider::parse(raw)
            .expect_err("unknown provider must not resolve to a default runtime");
        assert_eq!(&err.raw, raw, "error preserves the offending raw input");
        // Diagnostic lists the canonical set and never coerces to a default.
        assert!(
            err.to_string().contains(Provider::CANONICAL_LIST),
            "diagnostic for {raw:?} lists canonical providers"
        );
    }
}

#[test]
fn provider_from_str_matches_parse() {
    assert_eq!("grok".parse::<Provider>().expect("FromStr"), Provider::Grok);
    assert!("nope".parse::<Provider>().is_err());
}

#[test]
fn provider_alias_table_resolves_and_flags_deprecation() {
    for alias in Provider::ALIASES {
        assert_eq!(
            Provider::parse(alias.alias).expect("alias must parse"),
            alias.canonical
        );
        // The only shipped alias is a spelling variant, not a deprecated name.
        assert!(
            !alias.deprecated,
            "alias '{}' unexpectedly deprecated",
            alias.alias
        );
    }
}

#[test]
fn agent_role_serde_roundtrips_lowercase() {
    for (value, expected) in [
        (AgentRole::Reviewer, "\"reviewer\""),
        (AgentRole::Implementer, "\"implementer\""),
        (AgentRole::Planner, "\"planner\""),
    ] {
        let serialized = serde_json::to_string(&value).expect("serialize role");
        assert_eq!(serialized, expected);
        let parsed: AgentRole = serde_json::from_str(expected).expect("deserialize role");
        assert_eq!(parsed, value);
    }
}

#[test]
fn agent_loop_spec_yaml_includes_role_when_present() {
    let yaml = "instruction: hi\nrole: implementer\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.role, Some(AgentRole::Implementer));
}

#[test]
fn agent_loop_spec_yaml_role_is_optional() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.role, None);
}

#[test]
fn agent_loop_spec_defaults_to_cli_backend() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.backend, Backend::Cli);
}

#[test]
fn agent_loop_spec_proc_allowed_programs_defaults_to_none() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.proc_allowed_programs, None);
}

#[test]
fn agent_loop_spec_proc_allowed_programs_round_trips() {
    let yaml = "instruction: hi\nproc_allowed_programs:\n  - git\n  - rg\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(
        parsed.proc_allowed_programs,
        Some(vec!["git".to_string(), "rg".to_string()])
    );
    let reserialized = serde_yaml::to_string(&parsed).expect("serialize spec");
    let reparsed: AgentLoopSpec = serde_yaml::from_str(&reserialized).expect("re-parse spec");
    assert_eq!(reparsed.proc_allowed_programs, parsed.proc_allowed_programs);
}

#[test]
fn agent_loop_spec_proc_allowed_programs_accepts_empty_seq() {
    // Empty Some(vec![]) is meaningful: fail-closed when activity-scoped.
    let yaml = "instruction: hi\nproc_allowed_programs: []\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.proc_allowed_programs, Some(Vec::<String>::new()));
}

#[test]
fn groundhog_spec_mirrors_proc_allowed_programs_into_agent_loop() {
    let yaml = "instruction: hi\nproc_allowed_programs:\n  - git\n";
    let parsed: GroundhogSpec = serde_yaml::from_str(yaml).expect("parse groundhog spec");
    assert_eq!(parsed.proc_allowed_programs, Some(vec!["git".to_string()]));
    let derived = parsed.as_agent_loop_spec();
    assert_eq!(derived.proc_allowed_programs, parsed.proc_allowed_programs);
}

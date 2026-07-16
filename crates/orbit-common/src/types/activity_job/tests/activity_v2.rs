use super::super::activity_v2::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Exact vendored copy of the Constellation provider-resolution contract cases
/// (Polaris `rules/provider-resolution-cases.json`, ORB-10091) — byte-for-byte,
/// so the embedded `cases_sha256` stays valid and the Orbit resolver can be
/// asserted against the same rows as Worker and Bridge.
const CONTRACT_JSON: &str = include_str!("fixtures/provider_contract.json");

/// Pinned contract identity. Drift in the upstream contract changes the hash and
/// fails `provider_contract_hash_is_pinned` loudly, forcing a reviewed pin bump.
const PINNED_CONTRACT_VERSION: &str = "1.0.0";
const PINNED_CASES_SHA256: &str =
    "191058b435063a905943c9ef6779683e2b273ae0a0a338464419c74708d67cfe";

fn contract() -> Value {
    serde_json::from_str(CONTRACT_JSON).expect("parse vendored provider contract fixture")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn string_vec(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry.as_str().expect("string entry").to_string())
        .collect()
}

#[test]
fn provider_contract_hash_is_pinned() {
    let contract = contract();
    assert_eq!(
        contract["contract_version"].as_str(),
        Some(PINNED_CONTRACT_VERSION),
        "contract_version drifted; review and bump the pin",
    );
    // Recompute the hash the contract's way: sha256 over the canonical JSON of
    // the `cases` array (sorted keys, compact separators). serde_json::Value
    // serializes object keys sorted (no preserve_order feature) with `,`/`:`
    // compact separators — byte-equal to Python's
    // json.dumps(cases, sort_keys=True, separators=(",",":")).
    let canonical = serde_json::to_string(&contract["cases"]).expect("serialize cases");
    let recomputed = sha256_hex(canonical.as_bytes());
    assert_eq!(
        Some(recomputed.as_str()),
        contract["cases_sha256"].as_str(),
        "recomputed cases hash disagrees with the fixture's embedded value",
    );
    assert_eq!(
        recomputed, PINNED_CASES_SHA256,
        "cases hash drifted from the pinned contract revision",
    );
}

#[test]
fn provider_capability_predicates_match_contract() {
    let contract = contract();
    let canonical = string_vec(&contract["canonical_providers"]);
    let known = string_vec(&contract["known_providers"]);

    // Every canonical variant maps to exactly one known-provider row; adding a
    // `Provider` variant without a contract row (or vice-versa) fails here.
    assert_eq!(canonical.len(), 4, "canonical set is exactly four");
    assert_eq!(
        Provider::ALL.len(),
        known.len(),
        "Provider::ALL must mirror the contract's known_providers",
    );

    for name in &known {
        let provider =
            Provider::parse(name).unwrap_or_else(|_| panic!("known provider '{name}' must parse"));
        assert_eq!(provider.as_str(), name, "as_str round-trip for {name}");
        assert_eq!(provider.to_string(), *name, "Display round-trip for {name}");
        // Canonical == worker-executable; ollama/openai_compat are known Orbit
        // identities Worker cannot execute (the wider-set distinction).
        assert_eq!(
            provider.is_worker_executable(),
            canonical.contains(name),
            "is_worker_executable for {name}",
        );
        // openai_compat is the only HTTP-only id (no CLI runtime); claude is the
        // only id with an HTTP transport wired in Orbit today.
        assert_eq!(
            provider.has_cli_runtime(),
            name != "openai_compat",
            "has_cli_runtime for {name}",
        );
        assert_eq!(
            provider.has_http_transport(),
            name == "claude",
            "has_http_transport for {name}",
        );
    }
}

#[test]
fn provider_alias_table_matches_contract_and_signals_deprecation() {
    let contract = contract();
    let aliases = contract["aliases"].as_object().expect("aliases object");
    // Every deprecated alias in the contract resolves to its canonical id AND
    // carries an observable deprecation signal (normalized alias + canonical).
    for (alias, canonical) in aliases {
        let canonical = canonical.as_str().expect("canonical string");
        let identity = Provider::resolve_name(alias)
            .unwrap_or_else(|_| panic!("alias '{alias}' must resolve"));
        assert_eq!(
            identity.provider.as_str(),
            canonical,
            "alias {alias} -> {canonical}",
        );
        let deprecation = identity
            .deprecation
            .unwrap_or_else(|| panic!("alias '{alias}' must carry a deprecation signal"));
        assert_eq!(&deprecation.alias, alias, "signal alias for {alias}");
        assert_eq!(
            deprecation.canonical.as_str(),
            canonical,
            "signal canonical for {alias}",
        );
    }

    // Case-insensitive + whitespace: "  OpenAI " -> codex, signal alias "openai".
    let identity = Provider::resolve_name("  OpenAI ").expect("resolve mixed-case alias");
    assert_eq!(identity.provider, Provider::Codex);
    assert_eq!(
        identity.deprecation.expect("deprecation present").alias,
        "openai",
    );

    // Canonical ids and the non-deprecated spelling variant carry no signal.
    assert!(
        Provider::resolve_name("CODEX")
            .expect("canonical")
            .deprecation
            .is_none(),
    );
    assert!(
        Provider::resolve_name("openai-compat")
            .expect("spelling variant")
            .deprecation
            .is_none(),
    );
}

#[test]
fn provider_parse_rejects_unknown_without_fallback() {
    for raw in ["", "   ", "gpt", "claude-3", "grokk", "bogus"] {
        let err = Provider::parse(raw)
            .expect_err("unknown provider must not resolve to a default runtime");
        assert_eq!(err.raw, raw, "error preserves the offending raw input");
        assert!(
            err.to_string().contains(Provider::CANONICAL_LIST),
            "diagnostic for {raw:?} lists canonical providers",
        );
    }
}

#[test]
fn provider_from_str_matches_parse() {
    assert_eq!("grok".parse::<Provider>().expect("FromStr"), Provider::Grok);
    assert!("nope".parse::<Provider>().is_err());
}

/// The Orbit resolver must produce the same `(normalized_provider, source,
/// diagnostic, deprecation)` as every contract row whose `entry_point` is `any`
/// or `orbit` — the machine-checkable cross-repo parity assertion.
#[test]
fn resolver_matches_contract_cases_for_orbit_entry_points() {
    let contract = contract();
    let cases = contract["cases"].as_array().expect("cases array");
    let mut asserted = 0usize;

    for case in cases {
        let input = &case["input"];
        let id = case["id"].as_str().unwrap_or("<unknown>");

        // A repo runs the cases whose entry_point is `any` or its own (orbit);
        // worker/bridge-only rows are asserted by those repos.
        let entry_point = match input["entry_point"].as_str().expect("entry_point") {
            "any" | "orbit" => ProviderEntryPoint::Orbit,
            "worker" | "bridge" => continue,
            other => panic!("unexpected entry_point '{other}' in case {id}"),
        };

        let request = ProviderResolveRequest {
            entry_point,
            requested: input["requested"].as_str(),
            task_provider: input["task_provider"].as_str(),
            workspace_default: input["workspace_default"].as_str(),
            env_default: input["env_default"].as_str(),
            system_default: input["system_default"].as_str(),
            persisted_resolution: input["persisted_resolution"].as_str(),
            host_available: input["host_available"].as_bool(),
        };
        let got = Provider::resolve(&request);
        let expected = &case["expected"];

        assert_eq!(
            got.is_success(),
            expected["status"].as_str() == Some("success"),
            "status: {id}",
        );
        assert_eq!(
            got.normalized_provider.map(Provider::as_str),
            expected["normalized_provider"].as_str(),
            "normalized_provider: {id}",
        );
        assert_eq!(
            Some(got.source.as_str()),
            expected["source"].as_str(),
            "source: {id}",
        );
        assert_eq!(
            Some(got.diagnostic.as_str()),
            expected["diagnostic"].as_str(),
            "diagnostic: {id}",
        );
        match expected["deprecation"].as_object() {
            Some(obj) => {
                let deprecation = got
                    .deprecation
                    .as_ref()
                    .unwrap_or_else(|| panic!("expected deprecation signal: {id}"));
                assert_eq!(
                    Some(deprecation.alias.as_str()),
                    obj["alias"].as_str(),
                    "deprecation.alias: {id}",
                );
                assert_eq!(
                    Some(deprecation.canonical.as_str()),
                    obj["canonical"].as_str(),
                    "deprecation.canonical: {id}",
                );
            }
            None => assert!(got.deprecation.is_none(), "unexpected deprecation: {id}"),
        }
        asserted += 1;
    }

    // Guardrail: every any+orbit row ran (all but the three worker-only
    // unsupported/unavailable cases). Catches an accidental silent skip.
    assert_eq!(
        asserted,
        cases.len() - 3,
        "expected to run all any+orbit contract cases",
    );
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
fn agent_loop_spec_response_envelope_defaults_to_best_effort() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert!(!parsed.require_response_envelope);
}

#[test]
fn agent_loop_spec_required_response_envelope_round_trips() {
    let yaml = "instruction: hi\nrequire_response_envelope: true\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert!(parsed.require_response_envelope);
    let reserialized = serde_yaml::to_string(&parsed).expect("serialize spec");
    let reparsed: AgentLoopSpec = serde_yaml::from_str(&reserialized).expect("re-parse spec");
    assert!(reparsed.require_response_envelope);
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

use std::collections::BTreeSet;

use orbit_common::OrbitError;
use orbit_types::tool::{McpCapability, ToolSessionContext};

use super::super::callers::{
    CallersFile, DefaultGrant, SeedCaller, SessionCapabilityPolicy, load_callers,
    render_callers_seed, write_callers_seed,
};
use super::super::identity::McpSessionAuthority;

fn agent() -> BTreeSet<McpCapability> {
    BTreeSet::from([McpCapability::Agent])
}

fn operator() -> BTreeSet<McpCapability> {
    BTreeSet::from([McpCapability::Agent, McpCapability::Operator])
}

fn write(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("mcp-callers.toml");
    std::fs::write(&path, contents).expect("write callers");
    (dir, path)
}

#[test]
fn a_missing_file_is_a_valid_agent_only_configuration() {
    let dir = tempfile::tempdir().expect("temp dir");

    let file = load_callers(&dir.path().join("mcp-callers.toml")).expect("missing file loads");

    assert_eq!(file, CallersFile::default());
    assert_eq!(file.default, DefaultGrant::Agent);
    assert_eq!(file.resolve("hm_alpha").granted, agent());
}

#[test]
fn a_duplicate_machine_id_fails_the_whole_file_closed() {
    let (_dir, path) = write(
        r#"
default = "agent"

[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent"]

[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent", "operator"]
"#,
    );

    let error = load_callers(&path).expect_err("duplicate machine_id must fail closed");

    assert!(
        matches!(error, OrbitError::AmbiguousCaller(ref message) if message.contains("hm_alpha")),
        "expected ambiguous_caller, got {error:?}"
    );
}

#[test]
fn runner_is_not_a_grantable_capability() {
    let (_dir, path) = write(
        r#"
[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent", "runner"]
"#,
    );

    let error = load_callers(&path).expect_err("runner must not be grantable over a transport");

    assert!(
        matches!(error, OrbitError::InvalidInput(ref message) if message.contains("runner")),
        "expected an invalid-input refusal naming runner, got {error:?}"
    );
}

#[test]
fn a_malformed_file_is_never_served_as_if_absent() {
    for contents in [
        // Unknown key.
        "[[callers]]\nmachine_id = \"hm_alpha\"\ncapabilities = [\"agent\"]\nallow = true\n",
        // Empty capabilities.
        "[[callers]]\nmachine_id = \"hm_alpha\"\ncapabilities = []\n",
        // Unknown default.
        "default = \"operator\"\n",
        // Malformed machine_id.
        "[[callers]]\nmachine_id = \"not a machine\"\ncapabilities = [\"agent\"]\n",
        // Narrowing that is not a logical workspace ID.
        "[[callers]]\nmachine_id = \"hm_alpha\"\ncapabilities = [\"agent\"]\nworkspaces = [\"orbit\"]\n",
    ] {
        let (_dir, path) = write(contents);

        assert!(
            load_callers(&path).is_err(),
            "a malformed file must fail closed: {contents}"
        );
    }
}

#[test]
fn the_file_can_only_lower_what_argv_asked_for() {
    let (_dir, path) = write(
        r#"
[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent", "operator"]
"#,
    );
    let file = load_callers(&path).expect("callers");

    let agent_request =
        SessionCapabilityPolicy::from_grant(McpSessionAuthority::Agent, file.resolve("hm_alpha"));

    assert_eq!(
        agent_request.effective_for(None),
        agent(),
        "a caller granted operator that did not ask for it must still resolve to agent"
    );
}

#[test]
fn an_over_asking_caller_is_capped_by_the_destination() {
    let (_dir, path) = write(
        r#"
[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent"]
"#,
    );
    let file = load_callers(&path).expect("callers");

    let policy = SessionCapabilityPolicy::from_grant(
        McpSessionAuthority::Operator,
        file.resolve("hm_alpha"),
    );

    assert_eq!(policy.effective_for(None), agent());
}

#[test]
fn an_unmatched_caller_falls_to_the_default_not_to_argv() {
    let (_dir, path) = write(
        r#"
default = "deny"

[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent", "operator"]
"#,
    );
    let file = load_callers(&path).expect("callers");

    let policy =
        SessionCapabilityPolicy::from_grant(McpSessionAuthority::Operator, file.resolve("hm_beta"));

    assert!(
        policy.effective_for(None).is_empty(),
        "an unmatched caller under `default = deny` holds nothing"
    );
}

#[test]
fn a_workspaces_narrowing_is_evaluated_per_call() {
    let (_dir, path) = write(
        r#"
[[callers]]
machine_id = "hm_beta"
capabilities = ["agent", "operator"]
workspaces = ["ws_orbit"]
"#,
    );
    let file = load_callers(&path).expect("callers");
    let policy =
        SessionCapabilityPolicy::from_grant(McpSessionAuthority::Operator, file.resolve("hm_beta"));

    assert_eq!(policy.effective_for(Some("ws_orbit")), operator());
    assert_eq!(
        policy.effective_for(Some("ws_other")),
        agent(),
        "the same session holds only agent outside the workspaces its row lists"
    );
}

#[test]
fn a_narrowed_row_still_denies_elsewhere_under_a_deny_default() {
    let (_dir, path) = write(
        r#"
default = "deny"

[[callers]]
machine_id = "hm_beta"
capabilities = ["agent", "operator"]
workspaces = ["ws_orbit"]
"#,
    );
    let file = load_callers(&path).expect("callers");
    let policy =
        SessionCapabilityPolicy::from_grant(McpSessionAuthority::Operator, file.resolve("hm_beta"));

    assert!(
        policy.effective_for(Some("ws_other")).is_empty(),
        "narrowing falls back to the file default, which a deny default must not raise"
    );
}

#[test]
fn a_local_session_keeps_argv_authority_and_stamps_no_grant() {
    let policy = SessionCapabilityPolicy::local(McpSessionAuthority::Operator);
    let mut context = ToolSessionContext::default();

    policy.stamp(&mut context, Some("ws_orbit"));

    assert!(!policy.is_granted());
    assert_eq!(context.effective_capabilities, operator());
    assert_eq!(
        context.remote_caller_grant, None,
        "a local session has no destination-side statement to record"
    );
}

#[test]
fn a_granted_session_records_the_grant_beside_the_effective_set() {
    let (_dir, path) = write(
        r#"
[[callers]]
machine_id = "hm_alpha"
capabilities = ["agent"]
"#,
    );
    let file = load_callers(&path).expect("callers");
    let policy = SessionCapabilityPolicy::from_grant(
        McpSessionAuthority::Operator,
        file.resolve("hm_alpha"),
    );
    let mut context = ToolSessionContext::default();

    policy.stamp(&mut context, None);

    let grant = context
        .remote_caller_grant
        .expect("a remote-originated session records its grant");
    assert_eq!(grant.caller_machine_id, "hm_alpha");
    assert_eq!(grant.granted_capabilities, agent());
    assert_eq!(context.effective_capabilities, agent());
    assert!(grant.source.contains("mcp-callers.toml"));
}

#[test]
fn the_seeder_never_writes_an_operator_grant() {
    let seeded = render_callers_seed(&[
        SeedCaller {
            machine_id: "hm_alpha".to_string(),
            label: Some("daniels-mac-mini".to_string()),
        },
        SeedCaller {
            machine_id: "hm_beta".to_string(),
            label: None,
        },
    ]);

    assert!(!seeded.contains("\"operator\""));
    assert!(seeded.contains("machine_id   = \"hm_alpha\""));
    assert!(seeded.contains("label        = \"daniels-mac-mini\""));
    assert!(seeded.contains("capabilities = [\"agent\"]"));
}

#[test]
fn a_seeded_file_loads_and_grants_agent_only() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("mcp-callers.toml");
    let seeded = render_callers_seed(&[SeedCaller {
        machine_id: "hm_alpha".to_string(),
        label: None,
    }]);

    write_callers_seed(&path, &seeded).expect("seed writes");
    let file = load_callers(&path).expect("a seeded file must be valid");

    assert_eq!(file.resolve("hm_alpha").granted, agent());
    assert!(
        write_callers_seed(&path, &seeded).is_err(),
        "re-seeding must not overwrite an operator's statement"
    );
}

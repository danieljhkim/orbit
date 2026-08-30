//! Adoption of task-scoped GitHub reads for `ci-failure-remediation` [ORB-11070].
//!
//! DANI-10056 failed because the minted task carried no requirements and
//! `agent_implement` does not include GitHub reads. This file covers the
//! mint → production baseline union → dispatch export → preflight path
//! without a handwritten activity allowlist.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_engine::activity_job::load_activity_asset;
use orbit_engine::{RuntimeHost, V2AuditWriter, V2DispatchInput, dispatch_v2_activity};
use orbit_types::workflow::ActivityV2Spec;
use orbit_types::workflow::activity_job::{Provider, tool_allowed};
use serde_json::{Value, json};
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::adapter::command::override_activity_tools_for_test;
use crate::application::task::TaskAddParams;
use crate::auto_tasks::DEFAULT_AUTO_TASK_FILES;
use crate::bootstrap::activity::DEFAULT_ACTIVITY_FILES;

const CI_REMEDIATION_REQUIRED_TOOLS: [&str; 5] = [
    "github.auth.status",
    "github.pr.list",
    "github.run.list",
    "github.run.logs",
    "github.run.view",
];

const AGENT_IMPLEMENT_BASELINE: [&str; 4] = [
    "orbit.task.*",
    "orbit.friction.*",
    "orbit.search",
    "proc.spawn",
];

fn install_shipped_ci_remediation(runtime: &OrbitRuntime) {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "ci-failure-remediation")
        .expect("shipped ci-failure-remediation");
    let dest =
        crate::auto_tasks::definition_path(&runtime.paths().local_dir, "ci-failure-remediation");
    std::fs::create_dir_all(dest.parent().expect("auto_tasks parent"))
        .expect("create auto_tasks dir");
    std::fs::write(&dest, yaml).expect("install shipped definition");
}

fn agent_implement_spec() -> orbit_types::workflow::activity_job::AgentLoopSpec {
    let (_, yaml) = DEFAULT_ACTIVITY_FILES
        .iter()
        .find(|(name, _)| *name == "agent_implement")
        .expect("shipped agent_implement");
    let asset = load_activity_asset(yaml).expect("parse agent_implement");
    let ActivityV2Spec::AgentLoop(mut spec) = asset.spec.spec else {
        panic!("agent_implement must remain an agent_loop activity");
    };
    assert_eq!(spec.tools, AGENT_IMPLEMENT_BASELINE);
    spec.provider = Provider::Grok;
    spec.model = Some("grok-test".to_string());
    spec.wall_clock_timeout_seconds = 15;
    spec
}

fn add_ordinary_task(runtime: &OrbitRuntime) -> String {
    runtime
        .add_task(TaskAddParams {
            title: "Ordinary implementation".to_string(),
            description: "Neighboring task with empty required_tools.".to_string(),
            ..Default::default()
        })
        .expect("add ordinary task")
        .id
}

fn expected_effective_tools() -> Vec<String> {
    AGENT_IMPLEMENT_BASELINE
        .iter()
        .chain(CI_REMEDIATION_REQUIRED_TOOLS.iter())
        .map(|tool| (*tool).to_string())
        .collect()
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, contents).expect("write executable");
    let mut permissions = std::fs::metadata(path)
        .expect("executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod executable");
}

#[cfg(unix)]
fn locate_orbit_cli_binary() -> Option<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_orbit") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target_root.join(profile).join("orbit");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::var_os("ORBIT_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

#[test]
fn minted_ci_remediation_unions_agent_implement_baseline_and_denies_ordinary_github_reads() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    install_shipped_ci_remediation(&runtime);
    let minted = runtime
        .auto_task_mint("ci-failure-remediation")
        .expect("mint");
    let ordinary_id = add_ordinary_task(&runtime);
    let spec = agent_implement_spec();

    let ordinary = RuntimeHost::resolve_activity_tools(
        &runtime,
        std::slice::from_ref(&ordinary_id),
        &spec.tools,
    )
    .expect("resolve ordinary task");
    assert!(ordinary.requested_tools.is_empty());
    assert_eq!(ordinary.effective_tools, spec.tools);
    assert!(
        !tool_allowed("github.auth.status", &ordinary.effective_tools),
        "ordinary tasks must not inherit GitHub reads from agent_implement"
    );

    let remediated = RuntimeHost::resolve_activity_tools(
        &runtime,
        std::slice::from_ref(&minted.id),
        &spec.tools,
    )
    .expect("resolve minted CI-remediation task");
    assert_eq!(
        remediated.requested_tools,
        CI_REMEDIATION_REQUIRED_TOOLS.map(str::to_string)
    );
    assert_eq!(remediated.effective_tools, expected_effective_tools());

    let _denied = override_activity_tools_for_test(ordinary.effective_tools.clone());
    let error = runtime
        .execute_tool_command(
            "github.auth.status",
            json!({}),
            Some("grok".to_string()),
            Some("grok-test".to_string()),
        )
        .expect_err("ordinary implementation must not reach GitHub reads");
    assert!(
        matches!(error, OrbitError::PolicyDenied(_)),
        "expected policy_denied, got {error:?}"
    );
    drop(_denied);

    let stub = tempdir().expect("gh stub dir");
    let bin = stub.path().join("bin");
    std::fs::create_dir_all(&bin).expect("create stub bin");
    #[cfg(unix)]
    write_executable(&bin.join("gh"), "#!/bin/sh\nexit 0\n");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path = orbit_common::test_env::scoped([("PATH", Some(path.as_str()))]);
    let _allowed = override_activity_tools_for_test(remediated.effective_tools.clone());
    let preflight = runtime
        .execute_tool_command(
            "github.auth.status",
            json!({}),
            Some("grok".to_string()),
            Some("grok-test".to_string()),
        )
        .expect("CI-remediation preflight must reach tool execution");
    assert!(
        preflight
            .get("available")
            .and_then(Value::as_bool)
            .is_some()
    );
    assert!(
        preflight
            .get("authenticated")
            .and_then(Value::as_bool)
            .is_some()
    );
    assert!(
        preflight
            .get("detail")
            .and_then(Value::as_str)
            .is_some_and(|detail| !detail.is_empty()),
        "preflight must carry a quotable detail: {preflight}"
    );
}

#[cfg(unix)]
#[test]
fn mint_and_agent_implement_dispatch_export_the_computed_github_reads() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    install_shipped_ci_remediation(&runtime);
    let minted = runtime
        .auto_task_mint("ci-failure-remediation")
        .expect("mint");
    let ordinary_id = add_ordinary_task(&runtime);
    let spec = agent_implement_spec();

    let fixture = tempdir().expect("dispatch fixture");
    let grok = fixture.path().join("grok");
    let stdin_path = fixture.path().join("stdin.json");
    let tools_env_path = fixture.path().join("activity_tools.txt");
    let preflight_path = fixture.path().join("preflight.json");
    let gh_bin = fixture.path().join("bin");
    std::fs::create_dir_all(&gh_bin).expect("create gh stub dir");
    write_executable(&gh_bin.join("gh"), "#!/bin/sh\nexit 0\n");
    let nested_orbit = locate_orbit_cli_binary()
        .map(|path| format!("'{}'", path.display()))
        .unwrap_or_else(|| "''".to_string());
    write_executable(
        &grok,
        &format!(
            r#"#!/bin/sh
cat > '{stdin}'
printf '%s' "$ORBIT_ACTIVITY_TOOLS" > '{tools}'
if [ -n {orbit} ] && [ -x {orbit} ]; then
  {orbit} tool run github.auth.status > '{preflight}' || true
fi
printf '%s\n' '{{"schemaVersion":1,"status":"success","result":{{"ok":true}},"error":null}}'
"#,
            stdin = stdin_path.display(),
            tools = tools_env_path.display(),
            orbit = nested_orbit,
            preflight = preflight_path.display(),
        ),
    );

    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{inherited_path}", gh_bin.display());
    let grok_cmd = grok.display().to_string();
    let mut env_pairs = vec![
        ("ORBIT_V2_CLI_GROK", Some(grok_cmd.as_str())),
        ("PATH", Some(path.as_str())),
    ];
    let orbit_bin = locate_orbit_cli_binary().map(|path| path.display().to_string());
    if let Some(orbit_bin) = orbit_bin.as_deref() {
        env_pairs.push(("ORBIT_BIN", Some(orbit_bin)));
    }
    let _env = orbit_common::test_env::scoped(env_pairs);

    let audit_dir = tempdir().expect("audit dir");
    let audit = V2AuditWriter::with_disk_sinks(
        audit_dir.path(),
        Arc::new(orbit_store::Store::open_in_memory().expect("audit store")),
        "ws_test",
        "jrun-orb-11070",
        "grok:grok-test".to_string(),
        None,
    )
    .expect("audit writer");
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: "agent_implement",
        spec: &ActivityV2Spec::AgentLoop(spec.clone()),
        fs_profile: None,
        input: json!({
            "task_id": minted.id,
            "prompt": "Run the GitHub capability preflight."
        }),
        audit,
        run_id: "jrun-orb-11070",
        host: Some(&runtime),
    })
    .expect("dispatch agent_implement");
    assert!(
        outcome.success,
        "agent_implement dispatch failed: {:?}",
        outcome.message
    );

    let stdin = std::fs::read(&stdin_path).expect("read envelope");
    let stdin_text = String::from_utf8_lossy(&stdin);
    let envelope_text = stdin_text
        .rsplit_once("Execution envelope:\n")
        .map(|(_, rest)| rest.trim())
        .unwrap_or(stdin_text.trim());
    let envelope: Value = serde_json::from_str(envelope_text)
        .unwrap_or_else(|error| panic!("envelope JSON ({error}): {stdin_text}"));
    assert_eq!(
        envelope["required_tools"],
        json!(CI_REMEDIATION_REQUIRED_TOOLS)
    );
    assert_eq!(envelope["tools"], json!(expected_effective_tools()));
    let exported = std::fs::read_to_string(&tools_env_path).expect("read ORBIT_ACTIVITY_TOOLS");
    assert_eq!(exported, expected_effective_tools().join(","));

    let ordinary = RuntimeHost::resolve_activity_tools(
        &runtime,
        std::slice::from_ref(&ordinary_id),
        &spec.tools,
    )
    .expect("resolve ordinary");
    let _denied = override_activity_tools_for_test(ordinary.effective_tools);
    let error = runtime
        .execute_tool_command(
            "github.auth.status",
            json!({}),
            Some("grok".to_string()),
            Some("grok-test".to_string()),
        )
        .expect_err("ordinary task remains GitHub-denied");
    assert!(matches!(error, OrbitError::PolicyDenied(_)), "{error:?}");
    drop(_denied);

    let preflight = if preflight_path.is_file() {
        let bytes = std::fs::read(&preflight_path).expect("read nested preflight");
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(|value| {
                value.get("available").and_then(Value::as_bool).is_some()
                    && value
                        .get("authenticated")
                        .and_then(Value::as_bool)
                        .is_some()
            })
    } else {
        None
    };
    let preflight = preflight.unwrap_or_else(|| {
        let _allowed = override_activity_tools_for_test(
            envelope["tools"]
                .as_array()
                .expect("effective tools")
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
        );
        runtime
            .execute_tool_command(
                "github.auth.status",
                json!({}),
                Some("grok".to_string()),
                Some("grok-test".to_string()),
            )
            .expect("preflight from dispatched effective tools")
    });
    assert!(
        preflight
            .get("available")
            .and_then(Value::as_bool)
            .is_some()
    );
    assert!(
        preflight
            .get("authenticated")
            .and_then(Value::as_bool)
            .is_some()
    );
    assert!(
        preflight
            .get("detail")
            .and_then(Value::as_str)
            .is_some_and(|detail| !detail.is_empty()),
        "preflight must carry a quotable detail: {preflight}"
    );
}

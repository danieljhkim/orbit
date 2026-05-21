#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn quiet_inputs_exit_successfully_without_stdout() {
    let workspace = TestWorkspace::new();
    for (label, stdin) in [
        ("empty", ""),
        ("malformed", "not-json"),
        (
            "non-edit tool",
            r#"{"tool_name":"Bash","file_path":"src/lib.rs"}"#,
        ),
        ("missing path", r#"{"tool_name":"Edit"}"#),
    ] {
        let output = workspace.run_hook(stdin, &[("ORBIT_SESSION_ID", "quiet-session")], label);
        assert!(
            output.stdout.is_empty(),
            "{label} stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn global_skill_read_paths_exit_quietly_without_warning() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("Catch-all reminders stay scoped to the workspace", &["**"]);
    let absolute_skill = workspace.home.join(".orbit/skills/orbit/SKILL.md");

    for (label, path) in [
        (
            "tilde global skill",
            "~/.orbit/skills/orbit/SKILL.md".to_string(),
        ),
        (
            "absolute global skill",
            absolute_skill.to_string_lossy().to_string(),
        ),
    ] {
        let payload = json!({"tool_name": "Read", "path": path}).to_string();
        let output = workspace.run_hook(
            &payload,
            &[("ORBIT_SESSION_ID", "global-skill-session")],
            label,
        );
        assert!(
            output.stdout.is_empty(),
            "{label} stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("learning hook failed open"),
            "{label} stderr: {stderr}"
        );
    }
}

#[test]
fn matching_payload_emits_reminder_state_and_audit_event() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("Always keep hook reminders audited", &["src/**"]);
    let learning_id = learning["id"].as_str().expect("learning id");

    let output = workspace.run_hook(
        r#"{"tool_name":"Edit","file_path":"src/lib.rs"}"#,
        &[("ORBIT_SESSION_ID", "cold-session")],
        "hook cold",
    );
    let expected = format!(
        "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [{learning_id}] Always keep hook reminders audited\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);

    let state_path = workspace
        .work
        .join(".orbit/state/sessions/cold-session/learnings.json");
    let state: Value = serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
        .expect("state JSON");
    assert_eq!(state["count"], 1);
    assert_eq!(state["emitted_ids"], json!([learning_id]));

    let events = workspace.run_json(
        &["audit", "list", "--kind", "learning_injected", "--json"],
        "audit list",
    );
    let rows = events.as_array().expect("audit rows");
    assert_eq!(rows.len(), 1, "audit rows: {events}");
    let event = &rows[0];
    assert_eq!(event["tool_name"], "Edit");
    assert_eq!(event["target_type"], "learning_injected");
    assert_eq!(event["target_id"], "src/lib.rs");
    assert_eq!(event["session_id"], "cold-session");
    let arguments: Value =
        serde_json::from_str(event["arguments_json"].as_str().expect("arguments_json"))
            .expect("audit arguments JSON");
    assert_eq!(arguments["learning_ids"], json!([learning_id]));
}

#[test]
fn in_workspace_absolute_payload_still_emits_reminder() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("Absolute workspace paths still match", &["src/**"]);
    let learning_id = learning["id"].as_str().expect("learning id");
    let target = workspace.work.join("src/lib.rs");
    let payload =
        json!({"tool_name": "Read", "path": target.to_string_lossy().to_string()}).to_string();

    let output = workspace.run_hook(
        &payload,
        &[("ORBIT_SESSION_ID", "absolute-workspace-session")],
        "absolute workspace hook",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "- [{learning_id}] Absolute workspace paths still match"
        )),
        "stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("learning hook failed open"),
        "stderr: {stderr}"
    );
}

#[test]
fn repeated_payload_with_same_session_dedups_and_skips_second_audit() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("Dedup reminders within one session", &["src/**"]);
    let payload = r#"{"tool_name":"Edit","tool_input":{"file_path":"src/lib.rs"}}"#;

    let first = workspace.run_hook(payload, &[("ORBIT_SESSION_ID", "dedup-session")], "first");
    assert!(!first.stdout.is_empty());
    let second = workspace.run_hook(payload, &[("ORBIT_SESSION_ID", "dedup-session")], "second");
    assert!(second.stdout.is_empty());

    let events = workspace.run_json(
        &["audit", "list", "--kind", "learning_injected", "--json"],
        "audit list",
    );
    assert_eq!(events.as_array().expect("audit rows").len(), 1);
}

#[test]
fn per_call_cap_limits_rendered_learning_count() {
    let workspace = TestWorkspace::new();
    for idx in 0..6 {
        workspace.add_learning(&format!("cap learning {idx}"), &["src/**"]);
    }

    let output = workspace.run_hook(
        r#"{"tool_name":"Write","tool_input":{"filePath":"src/lib.rs"}}"#,
        &[
            ("ORBIT_SESSION_ID", "cap-session"),
            ("ORBIT_LEARNING_PER_CALL_CAP", "2"),
        ],
        "cap hook",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let rendered = stdout
        .lines()
        .filter(|line| line.starts_with("- ["))
        .count();
    assert_eq!(rendered, 2, "stdout: {stdout}");
}

#[test]
fn codex_format_emits_json_envelope_and_audit_event() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("Codex hook output must be JSON", &["src/**"]);
    let learning_id = learning["id"].as_str().expect("learning id");

    let output = workspace.run_hook_with_args(
        &["--format", "codex"],
        r#"{"tool_name":"Bash","tool_input":{"command":"cat src/lib.rs"}}"#,
        &[("ORBIT_SESSION_ID", "codex-format")],
        "codex format hook",
    );
    let rendered: Value = serde_json::from_slice(&output.stdout).expect("codex JSON stdout");
    assert_eq!(
        rendered["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("PreToolUse")
    );
    assert!(
        rendered["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additional context")
            .contains(&format!("- [{learning_id}] Codex hook output must be JSON"))
    );

    let events = workspace.run_json(
        &["audit", "list", "--kind", "learning_injected", "--json"],
        "audit list",
    );
    let event = &events.as_array().expect("audit rows")[0];
    assert_eq!(event["tool_name"], "Bash");
    assert_eq!(event["target_id"], "src/lib.rs");
}

#[test]
fn grok_format_matches_claude_plaintext_and_gemini_uses_json() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("Shared format test learning", &["src/**"]);
    let payload = r#"{"tool_name":"Read","path":"src/lib.rs"}"#;

    let claude = workspace.run_hook_with_args(
        &["--format", "claude"],
        payload,
        &[("ORBIT_SESSION_ID", "format-claude")],
        "claude format hook",
    );
    let grok = workspace.run_hook_with_args(
        &["--format", "grok"],
        payload,
        &[("ORBIT_SESSION_ID", "format-grok")],
        "grok format hook",
    );
    assert_eq!(claude.stdout, grok.stdout);

    let gemini = workspace.run_hook_with_args(
        &["--format", "gemini"],
        r#"{"tool_name":"read_file","tool_input":{"path":"src/lib.rs"}}"#,
        &[("ORBIT_SESSION_ID", "format-gemini")],
        "gemini format hook",
    );
    let rendered: Value = serde_json::from_slice(&gemini.stdout).expect("gemini JSON stdout");
    assert_eq!(
        rendered["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("BeforeTool")
    );
}

#[cfg(unix)]
#[test]
fn missing_session_uses_tmpdir_parent_pid_state_file() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("Fallback state path follows shell layout", &["src/**"]);
    let tmpdir = workspace.work.join("tmp");
    fs::create_dir_all(&tmpdir).expect("create tmpdir");

    let output = workspace.run_hook(
        r#"{"tool_name":"Read","path":"src/lib.rs"}"#,
        &[("TMPDIR", tmpdir.to_str().expect("tmpdir utf8"))],
        "fallback state",
    );
    assert!(!output.stdout.is_empty());

    let state_path = tmpdir.join(format!("orbit-learning-hook-{}.json", std::process::id()));
    let state: Value =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("read fallback state"))
            .expect("state JSON");
    assert_eq!(state["count"], 1);
    assert_eq!(
        state["emitted_ids"].as_array().expect("emitted ids").len(),
        1
    );
}

struct TestWorkspace {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        fs::create_dir_all(&home).expect("create home");
        fs::create_dir_all(&work).expect("create work");

        let workspace = Self {
            _temp: temp,
            home,
            work,
        };
        workspace.run(
            &["workspace", "init", "--name", "hook-pretooluse-test"],
            None,
            &[],
            "initialize workspace",
        );
        workspace
    }

    fn add_learning(&self, summary: &str, paths: &[&str]) -> Value {
        let mut args = vec!["learning", "add", "--summary", summary, "--json"];
        for path in paths {
            args.push("--path");
            args.push(*path);
        }
        self.run_json(&args, "add learning")
    }

    fn run_hook(&self, stdin: &str, envs: &[(&str, &str)], label: &str) -> Output {
        self.run(&["hook", "pretooluse"], Some(stdin), envs, label)
    }

    fn run_hook_with_args(
        &self,
        extra_args: &[&str],
        stdin: &str,
        envs: &[(&str, &str)],
        label: &str,
    ) -> Output {
        let mut args = vec!["hook", "pretooluse"];
        args.extend_from_slice(extra_args);
        self.run(&args, Some(stdin), envs, label)
    }

    fn run_json(&self, args: &[&str], label: &str) -> Value {
        let output = self.run(args, None, &[], label);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{label} produced invalid JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn run(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        envs: &[(&str, &str)],
        label: &str,
    ) -> Output {
        let output = run_orbit(&self.work, &self.home, args, stdin, envs);
        assert!(
            output.status.success(),
            "{label} failed\nargs: {args:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn run_orbit(
    cwd: &Path,
    home: &Path,
    args: &[&str],
    stdin: Option<&str>,
    envs: &[(&str, &str)],
) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_SESSION_ID")
        .env_remove("ORBIT_LEARNING_PER_CALL_CAP")
        .env_remove("ORBIT_LEARNING_SESSION_CAP")
        .env_remove("TMPDIR")
        .args(args);
    for (name, value) in envs {
        command.env(name, value);
    }
    if let Some(input) = stdin {
        command.write_stdin(input);
    }
    command.output().expect("run orbit")
}

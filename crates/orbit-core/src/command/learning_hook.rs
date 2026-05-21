use std::path::{Path, PathBuf};

use orbit_common::types::{
    LearningInjectionCaps, LearningInjectionState, LearningReminder, OrbitError,
};
use serde_json::Value;

use crate::OrbitRuntime;

pub const ORBIT_BIN_ENV: &str = "ORBIT_BIN";
pub const ORBIT_SESSION_ID_ENV: &str = "ORBIT_SESSION_ID";
pub const ORBIT_LEARNING_PER_CALL_CAP_ENV: &str = "ORBIT_LEARNING_PER_CALL_CAP";
pub const ORBIT_LEARNING_SESSION_CAP_ENV: &str = "ORBIT_LEARNING_SESSION_CAP";

pub type SessionLearningState = LearningInjectionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPayload {
    pub tool_name: String,
    pub target_path: String,
}

pub const CLAUDE_PRETOOLUSE_TOOLS: &[&str] = &["Edit", "Write", "Read"];
pub const CODEX_PRETOOLUSE_TOOLS: &[&str] = &["Bash", "apply_patch", "mcp"];
pub const GEMINI_PRETOOLUSE_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit",
    "replace",
    "run_shell_command",
    "Read",
    "Write",
    "Edit",
    "Bash",
];

pub fn parse_payload(stdin: &str) -> Option<HookPayload> {
    parse_payload_with_tools(stdin, CLAUDE_PRETOOLUSE_TOOLS)
}

pub fn parse_payload_with_tools(stdin: &str, accepted_tools: &[&str]) -> Option<HookPayload> {
    let value: Value = serde_json::from_str(stdin.trim()).ok()?;
    let object = value.as_object()?;
    let tool_name = string_field(&value, &["tool_name", "toolName"])?;
    if !tool_name_allowed(tool_name, accepted_tools) {
        return None;
    }

    let tool_input = object
        .get("tool_input")
        .or_else(|| object.get("toolInput"))
        .and_then(Value::as_object);
    let target_path = tool_input
        .and_then(first_path_in_object)
        .or_else(|| first_path_in_value(&value))
        .or_else(|| {
            tool_input
                .and_then(|input| {
                    ["patch", "diff"]
                        .iter()
                        .find_map(|key| input.get(*key).and_then(trimmed_string))
                })
                .and_then(path_from_patch)
        })
        .or_else(|| {
            tool_input
                .and_then(|input| {
                    ["command", "cmd"]
                        .iter()
                        .find_map(|key| input.get(*key).and_then(trimmed_string))
                })
                .and_then(path_from_shell_command)
        })?;

    Some(HookPayload {
        tool_name: tool_name.to_string(),
        target_path: target_path.to_string(),
    })
}

fn tool_name_allowed(tool_name: &str, accepted_tools: &[&str]) -> bool {
    accepted_tools.iter().any(|accepted| {
        tool_name == *accepted
            || (*accepted == "mcp" && tool_name.starts_with("mcp__"))
            || (*accepted == "mcp" && tool_name.starts_with("mcp."))
    })
}

fn first_path_in_value(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    first_path_in_object(object)
}

fn first_path_in_object(object: &serde_json::Map<String, Value>) -> Option<&str> {
    const STRING_KEYS: &[&str] = &[
        "file_path",
        "filePath",
        "path",
        "absolute_file_path",
        "absoluteFilePath",
        "fileName",
        "filename",
        "name",
    ];
    const ARRAY_KEYS: &[&str] = &[
        "file_paths",
        "filePaths",
        "paths",
        "files",
        "fileNames",
        "filenames",
        "absolute_file_paths",
        "absoluteFilePaths",
    ];

    STRING_KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(trimmed_string))
        .or_else(|| {
            ARRAY_KEYS.iter().find_map(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find_map(trimmed_string)
            })
        })
}

fn string_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(trimmed_string))
}

fn trimmed_string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn path_from_patch(patch: &str) -> Option<&str> {
    patch.lines().find_map(|line| {
        let line = line.trim();
        [
            "*** Update File: ",
            "*** Add File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix).map(str::trim))
        .filter(|value| !value.is_empty())
    })
}

fn path_from_shell_command(command: &str) -> Option<&str> {
    command
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .find(|token| looks_like_path(token))
}

fn looks_like_path(token: &str) -> bool {
    if token.starts_with('-') || matches!(token, "|" | ">" | "<" | "&&" | "||") {
        return false;
    }
    token.contains('/')
        || token.starts_with('.')
        || [
            ".rs", ".toml", ".json", ".md", ".yaml", ".yml", ".txt", ".sh", ".py",
        ]
        .iter()
        .any(|suffix| token.ends_with(suffix))
}

pub fn caps_from_env() -> LearningInjectionCaps {
    LearningInjectionCaps {
        per_call: cap_from_env(
            ORBIT_LEARNING_PER_CALL_CAP_ENV,
            orbit_common::types::DEFAULT_LEARNING_REMINDER_PER_CALL_CAP,
        ),
        per_session_hard: cap_from_env(
            ORBIT_LEARNING_SESSION_CAP_ENV,
            orbit_common::types::DEFAULT_LEARNING_REMINDER_SESSION_CAP,
        ),
    }
}

fn cap_from_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.max(1))
        .unwrap_or(default)
}

pub fn state_file_path(
    repo_root: &Path,
    session_id: Option<&str>,
    tmpdir: &Path,
    ppid: u32,
) -> PathBuf {
    match session_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(session_id) => repo_root
            .join(".orbit")
            .join("state")
            .join("sessions")
            .join(session_id)
            .join("learnings.json"),
        None => tmpdir.join(format!("orbit-learning-hook-{ppid}.json")),
    }
}

pub fn parse_state_json(raw: &str) -> SessionLearningState {
    let Ok(value) = serde_json::from_str::<Value>(raw.trim()) else {
        return SessionLearningState::new();
    };
    let Some(object) = value.as_object() else {
        return SessionLearningState::new();
    };
    let emitted_ids = object
        .get("emitted_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let count = object
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(emitted_ids.len());
    SessionLearningState { emitted_ids, count }
}

pub fn merge_state(
    mut prior: SessionLearningState,
    candidates: &[LearningReminder],
    caps: LearningInjectionCaps,
) -> (SessionLearningState, Vec<LearningReminder>) {
    let admitted = prior.admit_reminders(candidates, caps);
    (prior, admitted)
}

pub fn reminders_from_search_results(
    results: Vec<orbit_store::LearningSearchResult>,
) -> Vec<LearningReminder> {
    results
        .into_iter()
        .map(|result| LearningReminder {
            id: result.learning.id,
            summary: result.learning.summary,
            comments: Vec::new(),
        })
        .collect()
}

impl OrbitRuntime {
    pub fn learning_hook_target_is_searchable(
        &self,
        target_path: &str,
    ) -> Result<bool, OrbitError> {
        let normalized = target_path.trim().replace('\\', "/");
        if normalized == "~" || normalized.starts_with("~/") {
            return Ok(false);
        }

        crate::command::learning::learning_search_path_matches_workspace(
            &self.paths().repo_root,
            target_path,
        )
    }

    pub fn learning_hook_state_file_path(
        &self,
        session_id: Option<&str>,
        tmpdir: &Path,
        ppid: u32,
    ) -> PathBuf {
        state_file_path(&self.paths().repo_root, session_id, tmpdir, ppid)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use orbit_common::types::{LearningInjectionCaps, LearningReminder};

    use super::*;

    #[test]
    fn parse_payload_accepts_tool_and_path_variants() {
        let nested =
            parse_payload(r#"{"tool_name":"Edit","tool_input":{"file_path":" src/lib.rs "}}"#)
                .expect("nested payload");
        assert_eq!(nested.tool_name, "Edit");
        assert_eq!(nested.target_path, "src/lib.rs");

        let camel = parse_payload(r#"{"toolName":"Write","toolInput":{"filePath":"README.md"}}"#)
            .expect("camel payload");
        assert_eq!(camel.tool_name, "Write");
        assert_eq!(camel.target_path, "README.md");

        let top_level = parse_payload(r#"{"tool_name":"Read","path":"Cargo.toml"}"#)
            .expect("top-level payload");
        assert_eq!(top_level.tool_name, "Read");
        assert_eq!(top_level.target_path, "Cargo.toml");
    }

    #[test]
    fn parse_payload_rejects_malformed_irrelevant_or_pathless_payloads() {
        assert!(parse_payload("").is_none());
        assert!(parse_payload("not-json").is_none());
        assert!(parse_payload(r#"{"tool_name":"Bash","path":"src/lib.rs"}"#).is_none());
        assert!(parse_payload(r#"{"tool_name":"Edit","path":"   "}"#).is_none());
        assert!(parse_payload(r#"{"tool_name":"Edit"}"#).is_none());
    }

    #[test]
    fn parse_payload_with_tools_accepts_codex_path_shapes() {
        let bash = parse_payload_with_tools(
            r#"{"tool_name":"Bash","tool_input":{"command":"sed -n '1,20p' crates/orbit-core/src/lib.rs"}}"#,
            CODEX_PRETOOLUSE_TOOLS,
        )
        .expect("bash payload");
        assert_eq!(bash.tool_name, "Bash");
        assert_eq!(bash.target_path, "crates/orbit-core/src/lib.rs");

        let patch = parse_payload_with_tools(
            r#"{"tool_name":"apply_patch","tool_input":{"patch":"*** Begin Patch\n*** Update File: crates/orbit-cli/src/main.rs\n@@\n*** End Patch\n"}}"#,
            CODEX_PRETOOLUSE_TOOLS,
        )
        .expect("patch payload");
        assert_eq!(patch.tool_name, "apply_patch");
        assert_eq!(patch.target_path, "crates/orbit-cli/src/main.rs");

        let mcp = parse_payload_with_tools(
            r#"{"tool_name":"mcp__plugin_orbit__fs_read","tool_input":{"filePaths":["README.md","Cargo.toml"]}}"#,
            CODEX_PRETOOLUSE_TOOLS,
        )
        .expect("mcp payload");
        assert_eq!(mcp.target_path, "README.md");
    }

    #[test]
    fn state_file_path_matches_session_and_tmp_layouts() {
        let repo_root = Path::new("/repo");
        let tmpdir = Path::new("/tmp");
        assert_eq!(
            state_file_path(repo_root, Some("session-1"), tmpdir, 123),
            PathBuf::from("/repo/.orbit/state/sessions/session-1/learnings.json")
        );
        assert_eq!(
            state_file_path(repo_root, None, tmpdir, 123),
            PathBuf::from("/tmp/orbit-learning-hook-123.json")
        );
    }

    #[test]
    fn parse_state_json_defaults_malformed_or_missing_count() {
        assert_eq!(parse_state_json("not-json"), SessionLearningState::new());
        let state = parse_state_json(r#"{"emitted_ids":["L2","L1"]}"#);
        assert_eq!(state.count, 2);
        assert!(state.emitted_ids.contains("L1"));
        assert!(state.emitted_ids.contains("L2"));
    }

    #[test]
    fn merge_state_admits_cold_candidates_and_dedups_warm_state() {
        let candidates = reminders(&["L1", "L2"]);
        let caps = LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        };

        let (state, admitted) = merge_state(SessionLearningState::new(), &candidates, caps);
        assert_eq!(
            admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["L1", "L2"]
        );
        assert_eq!(state.count, 2);

        let (_state, admitted) = merge_state(state, &candidates, caps);
        assert!(admitted.is_empty());
    }

    #[test]
    fn merge_state_honors_per_call_and_session_caps() {
        let candidates = reminders(&["L1", "L2", "L3", "L4"]);
        let (state, admitted) = merge_state(
            SessionLearningState::new(),
            &candidates,
            LearningInjectionCaps {
                per_call: 2,
                per_session_hard: 20,
            },
        );
        assert_eq!(
            admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["L1", "L2"]
        );
        assert_eq!(state.count, 2);

        let seeded = SessionLearningState::seeded(["L1".to_string(), "L2".to_string()]);
        let (state, admitted) = merge_state(
            seeded,
            &candidates,
            LearningInjectionCaps {
                per_call: 5,
                per_session_hard: 2,
            },
        );
        assert!(admitted.is_empty());
        assert_eq!(state.count, 2);
    }

    #[test]
    fn caps_from_env_uses_shell_compatible_defaults_and_minimums() {
        let _guard = EnvGuard::set(&[
            (ORBIT_LEARNING_PER_CALL_CAP_ENV, None),
            (ORBIT_LEARNING_SESSION_CAP_ENV, None),
        ]);
        assert_eq!(
            caps_from_env(),
            LearningInjectionCaps {
                per_call: 5,
                per_session_hard: 20,
            }
        );
        drop(_guard);

        let _guard = EnvGuard::set(&[
            (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("2")),
            (ORBIT_LEARNING_SESSION_CAP_ENV, Some("0")),
        ]);
        assert_eq!(
            caps_from_env(),
            LearningInjectionCaps {
                per_call: 2,
                per_session_hard: 1,
            }
        );
        drop(_guard);

        let _guard = EnvGuard::set(&[
            (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("nope")),
            (ORBIT_LEARNING_SESSION_CAP_ENV, Some("also-nope")),
        ]);
        assert_eq!(
            caps_from_env(),
            LearningInjectionCaps {
                per_call: 5,
                per_session_hard: 20,
            }
        );
    }

    fn reminders(ids: &[&str]) -> Vec<LearningReminder> {
        ids.iter()
            .map(|id| LearningReminder {
                id: (*id).to_string(),
                summary: format!("summary {id}"),
                comments: Vec::new(),
            })
            .collect()
    }

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&'static str, Option<&str>)]) -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = values
                .iter()
                .map(|(name, _)| (*name, std::env::var(name).ok()))
                .collect::<Vec<_>>();
            for (name, value) in values {
                // SAFETY: EnvGuard serializes these process-wide mutations and restores them on drop.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                // SAFETY: EnvGuard holds the serialization lock until all saved values are restored.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }
}

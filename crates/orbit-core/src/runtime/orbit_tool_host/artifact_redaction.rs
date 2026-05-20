use std::collections::BTreeSet;

use orbit_common::types::{
    AuditEventStatus, OrbitError, audit_execution_id, normalize_optional_attribution_label,
};
use orbit_common::utility::redaction::{
    is_high_confidence_single_token_credential, redact_all, redact_home_dir,
    redact_sensitive_env_text,
};
use orbit_tools::OrbitBuiltinAction;
use serde_json::{Map, Value, json};

use crate::{AuditEventInsertParams, OrbitRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ArtifactRedactionKind {
    Env,
    Pattern,
    HomeDir,
}

impl ArtifactRedactionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Pattern => "pattern",
            Self::HomeDir => "home_dir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArtifactRedactionField {
    pub field_path: String,
    pub kinds: BTreeSet<ArtifactRedactionKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ArtifactRedactionReport {
    fields: Vec<ArtifactRedactionField>,
}

impl ArtifactRedactionReport {
    pub(super) fn redactions_applied(&self) -> bool {
        !self.fields.is_empty()
    }

    fn push(&mut self, field_path: String, kinds: BTreeSet<ArtifactRedactionKind>) {
        if !kinds.is_empty() {
            self.fields
                .push(ArtifactRedactionField { field_path, kinds });
        }
    }
}

pub(super) fn sanitize_tool_input(
    action: OrbitBuiltinAction,
    input: Value,
) -> Result<(Value, ArtifactRedactionReport), OrbitError> {
    let Some(policy) = policy_for_action(action) else {
        return Ok((input, ArtifactRedactionReport::default()));
    };
    let Value::Object(mut object) = input else {
        return Ok((input, ArtifactRedactionReport::default()));
    };

    let mut report = ArtifactRedactionReport::default();
    for field in policy.free_text_fields {
        sanitize_string_field(&mut object, field, field, TextMode::Free, &mut report)?;
    }
    for field in policy.free_text_arrays {
        sanitize_string_array_field(&mut object, field, field, TextMode::Free, &mut report)?;
    }
    for field in policy.path_fields {
        sanitize_string_field(&mut object, field, field, TextMode::PathOnly, &mut report)?;
    }
    for field in policy.path_arrays {
        sanitize_string_array_field(&mut object, field, field, TextMode::PathOnly, &mut report)?;
    }
    for nested in policy.nested_arrays {
        sanitize_nested_string_array_field(&mut object, nested, &mut report)?;
    }
    Ok((Value::Object(object), report))
}

pub(super) fn finish_tool_response(
    runtime: &OrbitRuntime,
    action: OrbitBuiltinAction,
    response: &mut Value,
    report: &ArtifactRedactionReport,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), OrbitError> {
    if !is_covered_mutating_action(action) {
        return Ok(());
    }
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "redactions_applied".to_string(),
            Value::Bool(report.redactions_applied()),
        );
    }
    if report.redactions_applied() {
        emit_audit_events(runtime, action, response, report, agent, model)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum TextMode {
    Free,
    PathOnly,
}

#[derive(Clone, Copy)]
struct NestedArrayPolicy {
    array_key: &'static str,
    field_key: &'static str,
    field_alias: Option<&'static str>,
    mode: TextMode,
}

struct ActionPolicy {
    free_text_fields: &'static [&'static str],
    free_text_arrays: &'static [&'static str],
    path_fields: &'static [&'static str],
    path_arrays: &'static [&'static str],
    nested_arrays: &'static [NestedArrayPolicy],
}

const LEARNING_NESTED: &[NestedArrayPolicy] = &[
    NestedArrayPolicy {
        array_key: "scope",
        field_key: "tags",
        field_alias: None,
        mode: TextMode::Free,
    },
    NestedArrayPolicy {
        array_key: "scope",
        field_key: "paths",
        field_alias: None,
        mode: TextMode::PathOnly,
    },
    NestedArrayPolicy {
        array_key: "evidence",
        field_key: "ref",
        field_alias: Some("reference"),
        mode: TextMode::Free,
    },
];

const TASK_ADD_NESTED: &[NestedArrayPolicy] = &[
    NestedArrayPolicy {
        array_key: "external_refs",
        field_key: "url",
        field_alias: None,
        mode: TextMode::PathOnly,
    },
    NestedArrayPolicy {
        array_key: "externalRefs",
        field_key: "url",
        field_alias: None,
        mode: TextMode::PathOnly,
    },
    NestedArrayPolicy {
        array_key: "external-refs",
        field_key: "url",
        field_alias: None,
        mode: TextMode::PathOnly,
    },
];

fn policy_for_action(action: OrbitBuiltinAction) -> Option<ActionPolicy> {
    match action {
        OrbitBuiltinAction::AdrAdd | OrbitBuiltinAction::AdrUpdate => Some(ActionPolicy {
            free_text_fields: &["title", "body"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::LearningAdd | OrbitBuiltinAction::LearningUpdate => {
            Some(ActionPolicy {
                free_text_fields: &["summary", "body"],
                free_text_arrays: &[],
                path_fields: &[],
                path_arrays: &[],
                nested_arrays: LEARNING_NESTED,
            })
        }
        OrbitBuiltinAction::LearningCommentAdd => Some(ActionPolicy {
            free_text_fields: &["body"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::TaskAdd => Some(ActionPolicy {
            free_text_fields: &["title", "description", "plan", "comment"],
            free_text_arrays: &["acceptance_criteria"],
            path_fields: &[],
            path_arrays: &["context_files", "context"],
            nested_arrays: TASK_ADD_NESTED,
        }),
        OrbitBuiltinAction::TaskUpdate => Some(ActionPolicy {
            free_text_fields: &[
                "title",
                "description",
                "plan",
                "execution_summary",
                "comment",
            ],
            free_text_arrays: &["acceptance_criteria"],
            path_fields: &[],
            path_arrays: &["context_files", "context"],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::TaskReject => Some(ActionPolicy {
            free_text_fields: &["note", "comment"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::TaskReopen => Some(ActionPolicy {
            free_text_fields: &["note", "comment"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::ReviewThreadAdd => Some(ActionPolicy {
            free_text_fields: &["body"],
            free_text_arrays: &[],
            path_fields: &["path"],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::ReviewThreadReply => Some(ActionPolicy {
            free_text_fields: &["body"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::FrictionAdd => Some(ActionPolicy {
            free_text_fields: &["body", "description"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::FrictionUpdate => Some(ActionPolicy {
            free_text_fields: &["body"],
            free_text_arrays: &[],
            path_fields: &[],
            path_arrays: &[],
            nested_arrays: &[],
        }),
        OrbitBuiltinAction::AdrSupersede | OrbitBuiltinAction::LearningSupersede => {
            Some(ActionPolicy {
                free_text_fields: &[],
                free_text_arrays: &[],
                path_fields: &[],
                path_arrays: &[],
                nested_arrays: &[],
            })
        }
        _ => None,
    }
}

fn is_covered_mutating_action(action: OrbitBuiltinAction) -> bool {
    matches!(
        action,
        OrbitBuiltinAction::AdrAdd
            | OrbitBuiltinAction::AdrUpdate
            | OrbitBuiltinAction::AdrSupersede
            | OrbitBuiltinAction::LearningAdd
            | OrbitBuiltinAction::LearningUpdate
            | OrbitBuiltinAction::LearningSupersede
            | OrbitBuiltinAction::LearningCommentAdd
            | OrbitBuiltinAction::TaskAdd
            | OrbitBuiltinAction::TaskUpdate
            | OrbitBuiltinAction::TaskReject
            | OrbitBuiltinAction::TaskReopen
            | OrbitBuiltinAction::ReviewThreadAdd
            | OrbitBuiltinAction::ReviewThreadReply
            | OrbitBuiltinAction::FrictionAdd
            | OrbitBuiltinAction::FrictionUpdate
    )
}

fn sanitize_string_field(
    object: &mut Map<String, Value>,
    key: &str,
    field_path: &str,
    mode: TextMode,
    report: &mut ArtifactRedactionReport,
) -> Result<(), OrbitError> {
    let Some(Value::String(raw)) = object.get(key) else {
        return Ok(());
    };
    let (sanitized, kinds) = sanitize_string(raw, field_path, mode)?;
    if sanitized != *raw {
        object.insert(key.to_string(), Value::String(sanitized));
        report.push(field_path.to_string(), kinds);
    }
    Ok(())
}

fn sanitize_string_array_field(
    object: &mut Map<String, Value>,
    key: &str,
    field_path: &str,
    mode: TextMode,
    report: &mut ArtifactRedactionReport,
) -> Result<(), OrbitError> {
    match object.get_mut(key) {
        Some(Value::String(raw)) => {
            let (sanitized, kinds) = sanitize_string(raw, field_path, mode)?;
            if sanitized != *raw {
                *raw = sanitized;
                report.push(field_path.to_string(), kinds);
            }
        }
        Some(Value::Array(items)) => {
            for (index, item) in items.iter_mut().enumerate() {
                let Value::String(raw) = item else {
                    continue;
                };
                let item_path = format!("{field_path}[{index}]");
                let (sanitized, kinds) = sanitize_string(raw, &item_path, mode)?;
                if sanitized != *raw {
                    *raw = sanitized;
                    report.push(item_path, kinds);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn sanitize_nested_string_array_field(
    object: &mut Map<String, Value>,
    policy: &NestedArrayPolicy,
    report: &mut ArtifactRedactionReport,
) -> Result<(), OrbitError> {
    if policy.array_key == "scope" {
        let Some(Value::Object(scope)) = object.get_mut("scope") else {
            return Ok(());
        };
        return sanitize_string_array_field(
            scope,
            policy.field_key,
            &format!("scope.{}", policy.field_key),
            policy.mode,
            report,
        );
    }

    let Some(Value::Array(items)) = object.get_mut(policy.array_key) else {
        return Ok(());
    };
    for (index, item) in items.iter_mut().enumerate() {
        let Value::Object(entry) = item else {
            continue;
        };
        let Some(key) = entry
            .contains_key(policy.field_key)
            .then_some(policy.field_key)
            .or_else(|| {
                policy
                    .field_alias
                    .filter(|alias| entry.contains_key(*alias))
            })
        else {
            continue;
        };
        let Some(Value::String(raw)) = entry.get(key) else {
            continue;
        };
        let field_path = format!("{}[{index}].{key}", policy.array_key);
        let (sanitized, kinds) = sanitize_string(raw, &field_path, policy.mode)?;
        if sanitized != *raw {
            entry.insert(key.to_string(), Value::String(sanitized));
            report.push(field_path, kinds);
        }
    }
    Ok(())
}

fn sanitize_string(
    raw: &str,
    field_path: &str,
    mode: TextMode,
) -> Result<(String, BTreeSet<ArtifactRedactionKind>), OrbitError> {
    match mode {
        TextMode::PathOnly => {
            let sanitized = redact_home_dir(raw);
            let mut kinds = BTreeSet::new();
            if sanitized != raw {
                kinds.insert(ArtifactRedactionKind::HomeDir);
            }
            Ok((sanitized, kinds))
        }
        TextMode::Free => {
            if is_high_confidence_single_token_credential(raw) {
                return Err(OrbitError::SensitiveInput {
                    field: field_path.to_string(),
                    reason: "whole-token credentials must not be persisted in Orbit artifacts"
                        .to_string(),
                });
            }
            let env_scrubbed = redact_sensitive_env_text(raw);
            let pattern_scrubbed = redact_all(raw);
            let sanitized = redact_home_dir(&pattern_scrubbed);
            let mut kinds = BTreeSet::new();
            if env_scrubbed != raw {
                kinds.insert(ArtifactRedactionKind::Env);
            }
            if pattern_scrubbed != env_scrubbed {
                kinds.insert(ArtifactRedactionKind::Pattern);
            }
            if sanitized != pattern_scrubbed {
                kinds.insert(ArtifactRedactionKind::HomeDir);
            }
            Ok((sanitized, kinds))
        }
    }
}

fn emit_audit_events(
    runtime: &OrbitRuntime,
    action: OrbitBuiltinAction,
    response: &Value,
    report: &ArtifactRedactionReport,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), OrbitError> {
    let tool_name = tool_name(action);
    let artifact = artifact_target(action, response)?;
    let actor = normalize_optional_attribution_label(model.or(agent), model)
        .unwrap_or_else(|| runtime.actor_label().to_string());

    for field in &report.fields {
        let redaction_kinds = field
            .kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();
        let payload = json!({
            "artifact_type": artifact.artifact_type,
            "artifact_id": artifact.artifact_id,
            "field_path": field.field_path,
            "actor": actor,
            "tool_name": tool_name,
            "redaction_kinds": redaction_kinds,
        });
        runtime.record_audit_event(&AuditEventInsertParams {
            execution_id: audit_execution_id("audit-artifact-redaction"),
            command: "artifact_redaction".to_string(),
            subcommand: Some("field".to_string()),
            tool_name: Some(tool_name.to_string()),
            target_type: Some(artifact.artifact_type.to_string()),
            target_id: Some(artifact.artifact_id.to_string()),
            role: actor.clone(),
            status: AuditEventStatus::Success,
            exit_code: 0,
            duration_ms: 0,
            working_directory: runtime.paths().repo_root.to_string_lossy().into_owned(),
            arguments_json: Some(payload.to_string()),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: None,
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
            task_id: artifact.task_id.map(ToOwned::to_owned),
            job_run_id: std::env::var("ORBIT_RUN_ID").ok().filter(|s| !s.is_empty()),
            activity_id: std::env::var("ORBIT_ACTIVITY_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            step_index: std::env::var("ORBIT_STEP_INDEX")
                .ok()
                .and_then(|s| s.parse().ok()),
        })?;
    }
    Ok(())
}

struct ArtifactTarget<'a> {
    artifact_type: &'static str,
    artifact_id: &'a str,
    task_id: Option<&'a str>,
}

fn artifact_target(
    action: OrbitBuiltinAction,
    response: &Value,
) -> Result<ArtifactTarget<'_>, OrbitError> {
    match action {
        OrbitBuiltinAction::AdrAdd
        | OrbitBuiltinAction::AdrUpdate
        | OrbitBuiltinAction::AdrSupersede => Ok(ArtifactTarget {
            artifact_type: "adr",
            artifact_id: response_string(response, "id")?,
            task_id: None,
        }),
        OrbitBuiltinAction::LearningAdd
        | OrbitBuiltinAction::LearningUpdate
        | OrbitBuiltinAction::LearningSupersede => Ok(ArtifactTarget {
            artifact_type: "learning",
            artifact_id: learning_response_id(action, response)?,
            task_id: None,
        }),
        OrbitBuiltinAction::LearningCommentAdd => Ok(ArtifactTarget {
            artifact_type: "learning_comment",
            artifact_id: response_string(response, "id")?,
            task_id: None,
        }),
        OrbitBuiltinAction::TaskAdd
        | OrbitBuiltinAction::TaskUpdate
        | OrbitBuiltinAction::TaskReject
        | OrbitBuiltinAction::TaskReopen => {
            let id = response_string(response, "id")?;
            Ok(ArtifactTarget {
                artifact_type: "task",
                artifact_id: id,
                task_id: Some(id),
            })
        }
        OrbitBuiltinAction::ReviewThreadAdd | OrbitBuiltinAction::ReviewThreadReply => {
            let id = response_string(response, "id")?;
            Ok(ArtifactTarget {
                artifact_type: "review_thread",
                artifact_id: id,
                task_id: Some(id),
            })
        }
        OrbitBuiltinAction::FrictionAdd | OrbitBuiltinAction::FrictionUpdate => {
            Ok(ArtifactTarget {
                artifact_type: "friction",
                artifact_id: response_string(response, "id")?,
                task_id: None,
            })
        }
        _ => Err(OrbitError::Execution(format!(
            "unsupported redaction audit action: {action:?}"
        ))),
    }
}

fn learning_response_id(action: OrbitBuiltinAction, response: &Value) -> Result<&str, OrbitError> {
    if action == OrbitBuiltinAction::LearningSupersede {
        return response
            .get("old")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OrbitError::Execution("learning supersede response missing old.id".to_string())
            });
    }
    response_string(response, "id")
}

fn response_string<'a>(response: &'a Value, field: &str) -> Result<&'a str, OrbitError> {
    response.get(field).and_then(Value::as_str).ok_or_else(|| {
        OrbitError::Execution(format!("redaction audit response missing string `{field}`"))
    })
}

fn tool_name(action: OrbitBuiltinAction) -> &'static str {
    match action {
        OrbitBuiltinAction::AdrAdd => "orbit.adr.add",
        OrbitBuiltinAction::AdrUpdate => "orbit.adr.update",
        OrbitBuiltinAction::AdrSupersede => "orbit.adr.supersede",
        OrbitBuiltinAction::LearningAdd => "orbit.learning.add",
        OrbitBuiltinAction::LearningUpdate => "orbit.learning.update",
        OrbitBuiltinAction::LearningSupersede => "orbit.learning.supersede",
        OrbitBuiltinAction::LearningCommentAdd => "orbit.learning.comment.add",
        OrbitBuiltinAction::TaskAdd => "orbit.task.add",
        OrbitBuiltinAction::TaskUpdate => "orbit.task.update",
        OrbitBuiltinAction::TaskReject => "orbit.task.reject",
        OrbitBuiltinAction::TaskReopen => "orbit.task.reopen",
        OrbitBuiltinAction::ReviewThreadAdd => "orbit.task.review_thread.add",
        OrbitBuiltinAction::ReviewThreadReply => "orbit.task.review_thread.reply",
        OrbitBuiltinAction::FrictionAdd => "orbit.friction.add",
        OrbitBuiltinAction::FrictionUpdate => "orbit.friction.update",
        _ => "orbit.unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use crate::runtime::orbit_tool_host::test_support::test_runtime;

    struct EnvVarGuard {
        _lock: MutexGuard<'static, ()>,
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var(name).ok();
            // SAFETY: this test guard serializes environment mutation and restores on drop.
            unsafe {
                std::env::set_var(name, value);
            }
            Self {
                _lock: lock,
                name,
                previous,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: the guard holds the serialization lock for the full mutation window.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.name, value),
                    None => std::env::remove_var(self.name),
                }
            }
        }
    }

    #[test]
    fn sanitizer_covers_task_free_text_paths_and_skipped_tags() {
        let home = std::env::var("HOME").expect("HOME for redaction test");
        let input = json!({
            "title": "uses sk-abcdefghijklmnopqrstuvwxyz",
            "description": "plain",
            "acceptance_criteria": ["keep ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456"],
            "context_files": [format!("{home}/repo/src/lib.rs"), "glob/[sk-abcdefghijklmnopqrstuvwxyz].rs"],
            "tags": ["sk-abcdefghijklmnopqrstuvwxyz"],
        });

        let (sanitized, report) =
            sanitize_tool_input(OrbitBuiltinAction::TaskAdd, input).expect("sanitize");

        assert!(report.redactions_applied());
        assert_eq!(sanitized["title"], "uses [REDACTED_SECRET]");
        assert!(
            sanitized["acceptance_criteria"][0]
                .as_str()
                .expect("criterion")
                .contains("[REDACTED_SECRET]")
        );
        assert_eq!(sanitized["context_files"][0], "~/repo/src/lib.rs");
        assert_eq!(
            sanitized["context_files"][1],
            "glob/[sk-abcdefghijklmnopqrstuvwxyz].rs"
        );
        assert_eq!(sanitized["tags"][0], "sk-abcdefghijklmnopqrstuvwxyz");
    }

    #[test]
    fn whole_token_credentials_are_rejected_for_representative_free_text_surfaces() {
        let cases = [
            (
                OrbitBuiltinAction::AdrAdd,
                json!({
                    "title": "sk-abcdefghijklmnopqrstuvwxyz",
                    "body": "Body",
                }),
            ),
            (
                OrbitBuiltinAction::LearningAdd,
                json!({
                    "summary": "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456",
                    "body": "Body",
                }),
            ),
            (
                OrbitBuiltinAction::TaskAdd,
                json!({
                    "title": "xoxb-0123456789",
                    "description": "Body",
                    "workspace": ".",
                }),
            ),
            (
                OrbitBuiltinAction::ReviewThreadAdd,
                json!({
                    "id": "ORB-00001",
                    "body": "sk-abcdefghijklmnopqrstuvwxyz",
                }),
            ),
            (
                OrbitBuiltinAction::FrictionAdd,
                json!({
                    "body": "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456",
                }),
            ),
        ];

        for (action, input) in cases {
            let err = sanitize_tool_input(action, input).expect_err("whole-token key rejected");

            assert!(
                matches!(err, OrbitError::SensitiveInput { .. }),
                "{action:?}: {err:?}"
            );
        }
    }

    #[test]
    fn learning_policy_treats_tags_as_text_and_paths_as_paths() {
        let home = std::env::var("HOME").expect("HOME for redaction test");
        let input = json!({
            "summary": "summary",
            "scope": {
                "tags": ["token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456"],
                "paths": [format!("{home}/repo/**/*.rs"), "[ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456]/**/*.rs"],
            },
            "evidence": [{"kind": "task", "ref": "see xoxb-0123456789"}],
        });

        let (sanitized, report) =
            sanitize_tool_input(OrbitBuiltinAction::LearningAdd, input).expect("sanitize");

        assert!(report.redactions_applied());
        assert_eq!(sanitized["scope"]["tags"][0], "token [REDACTED_SECRET]");
        assert_eq!(sanitized["scope"]["paths"][0], "~/repo/**/*.rs");
        assert_eq!(
            sanitized["scope"]["paths"][1],
            "[ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcd123456]/**/*.rs"
        );
        assert_eq!(sanitized["evidence"][0]["ref"], "see [REDACTED_SECRET]");
    }

    #[test]
    fn already_sanitized_input_is_idempotent() {
        let input = json!({
            "id": "ORB-00001",
            "execution_summary": "token [REDACTED_ENV]",
        });

        let (_sanitized, report) =
            sanitize_tool_input(OrbitBuiltinAction::TaskUpdate, input).expect("sanitize");

        assert!(!report.redactions_applied());
    }

    #[test]
    fn wrong_types_pass_through_to_existing_parsers() {
        let input = json!({
            "title": ["not", "a", "string"],
            "body": "Body",
        });

        let (sanitized, report) =
            sanitize_tool_input(OrbitBuiltinAction::AdrAdd, input.clone()).expect("sanitize");

        assert_eq!(sanitized, input);
        assert!(!report.redactions_applied());
    }

    #[test]
    fn dispatch_redacts_live_github_token_before_task_persistence_and_audits() {
        let token = "orbit-redaction-secret-value";
        let _env = EnvVarGuard::set("GITHUB_TOKEN", token);
        let (_root, runtime, _repo_root) = test_runtime();

        let output = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": format!("leaked {token}"),
                    "description": "body",
                    "workspace": ".",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task add succeeds");

        assert_eq!(output["redactions_applied"], true);
        let id = output["id"].as_str().expect("task id");
        let task = runtime.get_task(id).expect("task persisted");
        assert_eq!(task.title, "leaked [REDACTED_ENV]");
        assert!(!task.title.contains(token));

        let events = runtime
            .list_audit_events(None, Some("orbit.task.add".to_string()), None, None, 16)
            .expect("L20260517-9: same backing query as `orbit audit list --json`");
        let redaction_event = events
            .iter()
            .find(|event| event.command == "artifact_redaction")
            .expect("redaction audit event");
        let arguments = redaction_event
            .arguments_json
            .as_deref()
            .expect("redaction audit payload");
        assert!(arguments.contains("\"field_path\":\"title\""));
        assert!(arguments.contains("\"env\""));
        assert!(!arguments.contains(token));
    }

    #[test]
    fn adr_and_learning_dispatch_redact_live_github_token_in_persisted_fields() {
        let token = "orbit-adr-learning-secret-value";
        let _env = EnvVarGuard::set("GITHUB_TOKEN", token);
        let (_root, runtime, _repo_root) = test_runtime();

        let adr = runtime
            .execute_tool_command(
                "orbit.adr.add",
                json!({
                    "title": format!("decision {token}"),
                    "body": format!("## Context\n{token}"),
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("adr add succeeds");
        assert_eq!(adr["redactions_applied"], true);
        assert_eq!(adr["title"], "decision [REDACTED_ENV]");

        let learning = runtime
            .execute_tool_command(
                "orbit.learning.add",
                json!({
                    "summary": format!("summary {token}"),
                    "body": format!("body {token}"),
                    "scope": { "tags": [format!("tag {token}")] },
                    "evidence": [{ "kind": "task", "ref": format!("ref {token}") }],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("learning add succeeds");
        assert_eq!(learning["redactions_applied"], true);
        assert_eq!(learning["summary"], "summary [REDACTED_ENV]");
        assert_eq!(learning["body"], "body [REDACTED_ENV]");
        assert_eq!(learning["scope"]["tags"][0], "tag [redacted_env]");
        assert_eq!(learning["evidence"][0]["ref"], "ref [REDACTED_ENV]");
    }

    #[test]
    fn dispatch_marks_false_and_emits_no_audit_when_input_is_already_sanitized() {
        let (_root, runtime, _repo_root) = test_runtime();
        let created = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "plain",
                    "description": "body",
                    "workspace": ".",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task add succeeds");
        let id = created["id"].as_str().expect("task id");

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": id,
                    "execution_summary": "already [REDACTED_ENV]",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task update succeeds");

        assert_eq!(output["redactions_applied"], false);
        let events = runtime
            .list_audit_events(None, Some("orbit.task.update".to_string()), None, None, 16)
            .expect("L20260517-9: same backing query as `orbit audit list --json`");
        assert!(
            events
                .iter()
                .all(|event| event.command != "artifact_redaction"),
            "{events:?}"
        );
    }

    #[test]
    fn dispatch_adds_false_response_flags_for_each_covered_family() {
        let (_root, runtime, _repo_root) = test_runtime();

        let task = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "plain",
                    "description": "body",
                    "workspace": ".",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task add succeeds");
        assert_eq!(task["redactions_applied"], false);
        let task_id = task["id"].as_str().expect("task id");

        let adr = runtime
            .execute_tool_command(
                "orbit.adr.add",
                json!({
                    "title": "Decision",
                    "body": "## Context\nBody",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("adr add succeeds");
        assert_eq!(adr["redactions_applied"], false);

        let learning = runtime
            .execute_tool_command(
                "orbit.learning.add",
                json!({
                    "summary": "Always test the seam",
                    "body": "Body",
                    "scope": { "paths": ["crates/**"], "tags": ["testing"] },
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("learning add succeeds");
        assert_eq!(learning["redactions_applied"], false);

        let review = runtime
            .execute_tool_command(
                "orbit.task.review_thread.add",
                json!({
                    "id": task_id,
                    "body": "Please tighten this.",
                    "path": "src/lib.rs",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("review thread add succeeds");
        assert_eq!(review["redactions_applied"], false);

        let friction = runtime
            .execute_tool_command(
                "orbit.friction.add",
                json!({
                    "body": "Plain friction report.",
                    "tags": ["tooling"],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("friction add succeeds");
        assert_eq!(friction["redactions_applied"], false);
    }

    #[test]
    fn friction_body_update_is_sanitized_but_tags_are_verbatim() {
        let token = "orbit-friction-secret-value";
        let _env = EnvVarGuard::set("GITHUB_TOKEN", token);
        let (_root, runtime, _repo_root) = test_runtime();
        let tag = "sk-abcdefghijklmnopqrstuvwx";
        let frictions_root = runtime.data_root().join("frictions");
        fs::create_dir_all(&frictions_root).expect("frictions root");
        fs::write(
            frictions_root.join("tags.yaml"),
            format!("{tag}: \"synthetic test tag\"\n"),
        )
        .expect("custom friction taxonomy");
        let created = runtime
            .execute_tool_command(
                "orbit.friction.add",
                json!({
                    "body": "Plain friction report.",
                    "tags": [tag],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("friction add succeeds");
        assert_eq!(created["tags"], json!([tag]));

        let updated = runtime
            .execute_tool_command(
                "orbit.friction.update",
                json!({
                    "id": created["id"],
                    "body": format!("updated {token}"),
                    "tags": [tag],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("friction update succeeds");

        assert_eq!(updated["redactions_applied"], true);
        assert_eq!(updated["body"], "updated [REDACTED_ENV]");
        assert_eq!(updated["tags"], json!([tag]));
        assert!(
            !updated["body"].as_str().expect("body").contains(token),
            "{}",
            updated
        );
    }
}

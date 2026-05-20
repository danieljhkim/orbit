//! Host-side dispatch for the `orbit.adr.*` tool surface.
//!
//! See `docs/design/adr-artifact/2_design.md` §6 for the tool surface and §5
//! for the lifecycle rules enforced here.

use std::str::FromStr;

use orbit_common::types::{
    Adr, AdrStatus, AuditEventStatus, LegacyValidation, NotFoundKind, OrbitError,
    audit_execution_id, normalize_optional_attribution_label, optional_string,
    optional_string_alias, optional_string_list_alias, required_string,
};
use orbit_store::{AdrCreateParams, AdrDocumentUpdateParams};
use serde_json::{Value, json};

use crate::OrbitRuntime;

pub(super) fn add(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let title = required_string(&input, &["title"], "title")?;
    let owner = match optional_string(&input, "owner")? {
        Some(value) => value,
        None => actor_label(runtime, agent.as_deref(), model.as_deref()),
    };
    let body = required_string(&input, &["body"], "body")?;
    let related_features =
        optional_string_list_alias(&input, &["related_features", "features"])?.unwrap_or_default();
    let related_tasks =
        optional_string_list_alias(&input, &["related_tasks", "tasks"])?.unwrap_or_default();

    let adr = runtime.stores().adrs().add(AdrCreateParams {
        title,
        owner,
        related_features,
        related_tasks,
        body,
    })?;
    runtime.record_id_allocation_audit("adr", &adr.id)?;
    Ok(adr_to_json(&adr))
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = optional_string(&input, "id")?;
    let legacy_id = optional_string_alias(&input, &["legacy_id", "legacyId"])?;

    let (id_value, by_legacy) = match (id, legacy_id) {
        (Some(id), None) => (id, false),
        (None, Some(legacy)) => (legacy, true),
        (Some(_), Some(_)) => {
            return Err(OrbitError::InvalidInput(
                "specify exactly one of `id` or `legacy_id`".to_string(),
            ));
        }
        (None, None) => {
            return Err(OrbitError::InvalidInput(
                "missing required field: `id` or `legacy_id`".to_string(),
            ));
        }
    };

    let adrs = runtime.stores().adrs();
    let adr = if by_legacy {
        let matches = adrs.list_filtered(None, None, None, None, Some(&id_value), None)?;
        if matches.len() > 1 {
            return Err(OrbitError::InvalidInput(format!(
                "legacy_id `{id_value}` resolves to {} ADRs; specify the canonical id",
                matches.len()
            )));
        }
        matches
            .into_iter()
            .next()
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, id_value.clone()))?
    } else {
        adrs.get(&id_value)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, id_value.clone()))?
    };
    Ok(adr_to_json(&adr))
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let status = optional_string(&input, "status")?
        .map(|raw| parse_status_filter(&raw))
        .transpose()?;
    let owner = optional_string(&input, "owner")?;
    let feature = optional_string(&input, "feature")?;
    let task_id = optional_string_alias(&input, &["task_id", "task"])?;
    let legacy_id = optional_string_alias(&input, &["legacy_id", "legacyId"])?;
    let validation_warned =
        super::input::optional_bool_alias(&input, &["validation_warned", "validation"])?;

    let adrs = runtime.stores().adrs().list_filtered(
        status,
        owner.as_deref(),
        feature.as_deref(),
        task_id.as_deref(),
        legacy_id.as_deref(),
        validation_warned,
    )?;
    Ok(Value::Array(adrs.iter().map(adr_to_json).collect()))
}

pub(super) fn update(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let adrs = runtime.stores().adrs();
    let existing = adrs
        .get(&id)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, id.clone()))?;

    let new_status = optional_string(&input, "status")?
        .map(|raw| AdrStatus::from_str(&raw).map_err(OrbitError::InvalidInput))
        .transpose()?;

    let fields = AdrDocumentUpdateParams {
        title: optional_string(&input, "title")?,
        owner: optional_string(&input, "owner")?,
        body: optional_string(&input, "body")?,
        related_features: optional_string_list_alias(&input, &["related_features", "features"])?,
        related_tasks: optional_string_list_alias(&input, &["related_tasks", "tasks"])?,
        supersedes: optional_string_list_alias(&input, &["supersedes"])?,
        superseded_by: None, // see below: clients use orbit.adr.supersede
        legacy_ids: optional_string_list_alias(&input, &["legacy_ids", "legacyIds"])?,
        validation_warnings: optional_string_list_alias(
            &input,
            &["validation_warnings", "validationWarnings"],
        )?,
        legacy_validation: optional_string(&input, "legacy_validation")?
            .map(|raw| LegacyValidation::from_str(&raw).map_err(OrbitError::InvalidInput))
            .transpose()?,
    };

    if has_document_changes(&fields) {
        adrs.update_document(&id, &fields)?;
    }

    if let Some(target) = new_status {
        let from = existing.status;
        match target {
            AdrStatus::Superseded => {
                return Err(OrbitError::InvalidInput(format!(
                    "direct status -> superseded is rejected; use orbit.adr.supersede (id={id})"
                )));
            }
            AdrStatus::Accepted if from == AdrStatus::Proposed => {
                // The lifecycle rule: proposed -> accepted requires
                // non-empty related_tasks on the resulting record.
                let resulting_tasks: &[String] = match fields.related_tasks.as_deref() {
                    Some(slice) => slice,
                    None => existing.related_tasks.as_slice(),
                };
                if resulting_tasks.is_empty() {
                    return Err(OrbitError::AdrInvalidTransition(format!(
                        "{id}: proposed -> accepted requires non-empty related_tasks"
                    )));
                }
            }
            _ => {}
        }

        if target != from {
            adrs.update_status(&id, target)?;
            let task_id = if target == AdrStatus::Accepted {
                fields
                    .related_tasks
                    .as_deref()
                    .and_then(|tasks| tasks.first().cloned())
                    .or_else(|| existing.related_tasks.first().cloned())
            } else {
                None
            };
            record_transition_audit(
                runtime,
                "orbit.adr.update",
                &id,
                from,
                target,
                agent.as_deref(),
                model.as_deref(),
                task_id.as_deref(),
                None,
            )?;
        }
    }

    let updated = adrs
        .get(&id)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, id.clone()))?;
    Ok(adr_to_json(&updated))
}

pub(super) fn supersede(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let old_id = required_string(&input, &["old_id", "old", "oldId"], "old_id")?;
    let new_id = required_string(&input, &["new_id", "new", "newId"], "new_id")?;
    let adrs = runtime.stores().adrs();
    let before = adrs
        .get(&old_id)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, old_id.clone()))?;
    adrs.supersede(&old_id, &new_id)?;
    record_transition_audit(
        runtime,
        "orbit.adr.supersede",
        &old_id,
        before.status,
        AdrStatus::Superseded,
        agent.as_deref(),
        model.as_deref(),
        None,
        Some(&new_id),
    )?;
    let updated = adrs
        .get(&old_id)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, old_id.clone()))?;
    Ok(adr_to_json(&updated))
}

fn parse_status_filter(raw: &str) -> Result<AdrStatus, OrbitError> {
    AdrStatus::from_str(raw).map_err(OrbitError::InvalidInput)
}

fn has_document_changes(fields: &AdrDocumentUpdateParams) -> bool {
    fields.title.is_some()
        || fields.owner.is_some()
        || fields.body.is_some()
        || fields.related_features.is_some()
        || fields.related_tasks.is_some()
        || fields.supersedes.is_some()
        || fields.legacy_ids.is_some()
        || fields.validation_warnings.is_some()
        || fields.legacy_validation.is_some()
}

fn actor_label(runtime: &OrbitRuntime, agent: Option<&str>, model: Option<&str>) -> String {
    normalize_optional_attribution_label(model.or(agent), model)
        .unwrap_or_else(|| runtime.actor_label().to_string())
}

fn adr_to_json(adr: &Adr) -> Value {
    json!({
        "id": adr.id,
        "title": adr.title,
        "status": adr.status.cli_name(),
        "owner": adr.owner,
        "created_at": adr.created_at.to_rfc3339(),
        "accepted_at": adr.accepted_at.map(|ts| ts.to_rfc3339()),
        "last_updated": adr.last_updated.to_rfc3339(),
        "related_features": adr.related_features,
        "related_tasks": adr.related_tasks,
        "supersedes": adr.supersedes,
        "superseded_by": adr.superseded_by,
        "legacy_ids": adr.legacy_ids,
        "validation_warnings": adr.validation_warnings,
        "legacy_validation": adr.legacy_validation.to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn record_transition_audit(
    runtime: &OrbitRuntime,
    tool_name: &str,
    adr_id: &str,
    from: AdrStatus,
    to: AdrStatus,
    agent: Option<&str>,
    model: Option<&str>,
    task_id: Option<&str>,
    supersede_new: Option<&str>,
) -> Result<(), OrbitError> {
    let actor = actor_label(runtime, agent, model);
    let mut payload = json!({
        "adr_id": adr_id,
        "from_status": from.cli_name(),
        "to_status": to.cli_name(),
        "actor": actor,
    });
    if let Some(task) = task_id {
        payload["task_id"] = Value::String(task.to_string());
    }
    if let Some(new) = supersede_new {
        payload["supersede_new_id"] = Value::String(new.to_string());
    }

    let arguments_json = serde_json::to_string(&payload)
        .map_err(|error| OrbitError::Execution(format!("serialize adr audit payload: {error}")))?;

    runtime.record_audit_event(&crate::AuditEventInsertParams {
        execution_id: audit_execution_id("audit-adr-transition"),
        command: "adr".to_string(),
        subcommand: Some("transition".to_string()),
        tool_name: Some(tool_name.to_string()),
        target_type: Some("adr_transition".to_string()),
        target_id: Some(adr_id.to_string()),
        role: "admin".to_string(),
        status: AuditEventStatus::Success,
        exit_code: 0,
        duration_ms: 0,
        working_directory: runtime.paths().repo_root.to_string_lossy().into_owned(),
        arguments_json: Some(arguments_json),
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: None,
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        task_id: task_id.map(ToOwned::to_owned),
        job_run_id: std::env::var("ORBIT_RUN_ID").ok().filter(|s| !s.is_empty()),
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|s| s.parse().ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitRuntime;
    use crate::runtime::orbit_tool_host::test_support::test_runtime;
    use orbit_common::types::{LearningScope, NotFoundKind};
    use orbit_store::LearningCreateParams;
    use tempfile::tempdir;

    fn assert_adr_field(value: &Value, field: &str, expected: &str) {
        let actual = value
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected `{field}` in response: {value}"));
        assert_eq!(actual, expected, "field `{field}`");
    }

    #[test]
    fn add_creates_proposed_adr_with_assigned_id() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let response = add(
            &runtime,
            json!({
                "title": "Initial decision",
                "owner": "claude",
                "body": "## Context\nA body.",
            }),
            Some("claude".to_string()),
            None,
        )
        .expect("add");
        assert_adr_field(&response, "id", "ADR-0001");
        assert_adr_field(&response, "status", "proposed");
        assert_adr_field(&response, "owner", "claude");
    }

    #[test]
    fn add_accepts_empty_related_tasks() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let response = add(
            &runtime,
            json!({
                "title": "Open question",
                "owner": "claude",
                "body": "Body",
                "related_tasks": [],
            }),
            None,
            None,
        )
        .expect("add");
        assert!(
            response["related_tasks"].as_array().unwrap().is_empty(),
            "related_tasks empty per ADR-008"
        );
    }

    #[test]
    fn add_adr_and_learning_emit_id_allocation_audit_events() {
        let (_guard, runtime, repo_root) = test_runtime();
        let adr = add(
            &runtime,
            json!({"title": "Audited", "owner": "codex", "body": "Body"}),
            None,
            None,
        )
        .expect("add adr");
        let adr_id = adr["id"].as_str().expect("adr id").to_string();
        let learning = runtime
            .create_learning(LearningCreateParams {
                summary: "Audited learning".to_string(),
                scope: LearningScope::default(),
                body: String::new(),
                evidence: Vec::new(),
                created_by: Some("codex".to_string()),
                priority: None,
            })
            .expect("add learning");

        let events = runtime
            .list_audit_events_with_kind(
                None,
                None,
                Some("id_allocation".to_string()),
                None,
                None,
                10,
            )
            .expect("audit list");
        let repo_root = repo_root.to_string_lossy().to_string();
        let payloads = events
            .iter()
            .map(|event| {
                serde_json::from_str::<Value>(
                    event.arguments_json.as_deref().expect("arguments json"),
                )
                .expect("payload")
            })
            .collect::<Vec<_>>();

        assert!(payloads.iter().any(|payload| {
            payload["kind"].as_str() == Some("adr")
                && payload["id"].as_str() == Some(adr_id.as_str())
                && payload["worktree_root"].as_str() == Some(repo_root.as_str())
        }));
        assert!(payloads.iter().any(|payload| {
            payload["kind"].as_str() == Some("learning")
                && payload["id"].as_str() == Some(learning.id.as_str())
                && payload["worktree_root"].as_str() == Some(repo_root.as_str())
        }));
    }

    #[test]
    fn add_adr_and_learning_keep_body_files_in_shared_root_when_local_root_differs() {
        let root = tempdir().expect("tempdir");
        let global_root = root.path().join("global");
        let shared_repo = root.path().join("repo");
        let local_worktree = root.path().join("linked-worktree");
        let shared_root = shared_repo.join(".orbit");
        let local_root = local_worktree.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("global root");
        std::fs::create_dir_all(&shared_root).expect("shared root");
        std::fs::create_dir_all(&local_root).expect("local root");
        let runtime = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &local_root)
            .expect("runtime");

        let adr = add(
            &runtime,
            json!({"title": "Shared ADR", "owner": "codex", "body": "Body"}),
            None,
            None,
        )
        .expect("add adr");
        let adr_id = adr["id"].as_str().expect("adr id");
        let learning = runtime
            .create_learning(LearningCreateParams {
                summary: "Shared learning".to_string(),
                scope: LearningScope::default(),
                body: String::new(),
                evidence: Vec::new(),
                created_by: Some("codex".to_string()),
                priority: None,
            })
            .expect("add learning");

        assert!(
            shared_root
                .join("adrs/proposed")
                .join(adr_id)
                .join("body.md")
                .is_file()
        );
        assert!(
            shared_root
                .join("learnings")
                .join(&learning.id)
                .join("learning.yaml")
                .is_file()
        );
        assert!(!local_root.join("adrs").exists());
        assert!(!local_root.join("learnings").exists());
    }

    #[test]
    fn show_resolves_by_id() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        let id = created["id"].as_str().unwrap().to_string();
        let response = show(&runtime, json!({"id": id.clone()})).expect("show");
        assert_adr_field(&response, "id", &id);
    }

    #[test]
    fn show_resolves_by_legacy_id() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        let id = created["id"].as_str().unwrap().to_string();
        update(
            &runtime,
            json!({"id": id.clone(), "legacy_ids": ["activity-job/ADR-039"]}),
            None,
            None,
        )
        .expect("set legacy");

        let response =
            show(&runtime, json!({"legacy_id": "activity-job/ADR-039"})).expect("show by legacy");
        assert_adr_field(&response, "id", &id);
    }

    #[test]
    fn show_rejects_both_id_and_legacy_id() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let err = show(
            &runtime,
            json!({"id": "ADR-0001", "legacy_id": "foo/ADR-1"}),
        )
        .expect_err("rejects ambiguous input");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn show_missing_returns_not_found() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let err = show(&runtime, json!({"id": "ADR-9999"})).expect_err("missing");
        assert!(matches!(
            err,
            OrbitError::NotFound {
                kind: NotFoundKind::Adr,
                ..
            }
        ));
    }

    #[test]
    fn list_filters_by_status_descending() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let _a = add(
            &runtime,
            json!({"title": "A", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("a");
        let b = add(
            &runtime,
            json!({"title": "B", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("b");
        update(
            &runtime,
            json!({
                "id": b["id"],
                "status": "accepted",
                "related_tasks": ["T20260511-1"],
            }),
            None,
            None,
        )
        .expect("accept b");

        let listed = list(&runtime, json!({"status": "accepted"})).expect("list accepted");
        let arr = listed.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], b["id"]);
    }

    #[test]
    fn update_proposed_to_accepted_without_tasks_is_rejected() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        let err = update(
            &runtime,
            json!({"id": created["id"], "status": "accepted"}),
            None,
            None,
        )
        .expect_err("rejects accept without tasks");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
    }

    #[test]
    fn update_proposed_to_accepted_with_tasks_succeeds() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        let response = update(
            &runtime,
            json!({
                "id": created["id"],
                "status": "accepted",
                "related_tasks": ["T20260511-1"],
            }),
            None,
            None,
        )
        .expect("accept");
        assert_adr_field(&response, "status", "accepted");
    }

    #[test]
    fn update_accepted_to_proposed_is_rejected() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        update(
            &runtime,
            json!({
                "id": created["id"],
                "status": "accepted",
                "related_tasks": ["T20260511-1"],
            }),
            None,
            None,
        )
        .expect("accept");
        let err = update(
            &runtime,
            json!({"id": created["id"], "status": "proposed"}),
            None,
            None,
        )
        .expect_err("rejects regression");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
    }

    #[test]
    fn update_direct_to_superseded_is_rejected() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let created = add(
            &runtime,
            json!({"title": "T", "owner": "claude", "body": "B"}),
            None,
            None,
        )
        .expect("add");
        let err = update(
            &runtime,
            json!({"id": created["id"], "status": "superseded"}),
            None,
            None,
        )
        .expect_err("rejects direct superseded write");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn supersede_writes_bidirectional_edges() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let old = add(
            &runtime,
            json!({"title": "Old", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("old");
        let new = add(
            &runtime,
            json!({"title": "New", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("new");
        update(
            &runtime,
            json!({
                "id": new["id"],
                "status": "accepted",
                "related_tasks": ["T20260511-1"],
            }),
            None,
            None,
        )
        .expect("accept new");

        let result = supersede(
            &runtime,
            json!({"old_id": old["id"], "new_id": new["id"]}),
            None,
            None,
        )
        .expect("supersede");
        assert_adr_field(&result, "status", "superseded");
        assert_eq!(result["superseded_by"], new["id"]);

        let after_new = show(&runtime, json!({"id": new["id"]})).expect("show new");
        let supersedes = after_new["supersedes"].as_array().unwrap();
        assert_eq!(supersedes.len(), 1);
        assert_eq!(supersedes[0], old["id"]);
    }

    #[test]
    fn supersede_target_not_accepted_returns_invalid_transition() {
        let (_guard, runtime, _repo_root) = test_runtime();
        let old = add(
            &runtime,
            json!({"title": "Old", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("old");
        let new = add(
            &runtime,
            json!({"title": "New", "owner": "claude", "body": "b"}),
            None,
            None,
        )
        .expect("new");
        let err = supersede(
            &runtime,
            json!({"old_id": old["id"], "new_id": new["id"]}),
            None,
            None,
        )
        .expect_err("rejects non-accepted target");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
    }
}

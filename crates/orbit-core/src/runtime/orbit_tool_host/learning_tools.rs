//! Host-side dispatch for the `orbit.learning.*` tool surface.
//!
//! See `docs/design/project-learnings/2_design.md` §5 for the tool surface
//! and §3 / §7 for the matching and staleness rules enforced here. Writes
//! go through `runtime.stores().learnings()` which is the same handle the
//! CLI uses, so the file-system and SQLite-index state stays consistent
//! regardless of entry point.

use std::str::FromStr;

use orbit_common::types::{
    EvidenceKind, LearningEvidence, LearningScope, LearningStatus, NotFoundKind, OrbitError,
    optional_string, optional_string_alias, required_string,
};
use orbit_store::{
    LearningCreateParams, LearningListEntry, LearningUpdateParams, RemoteArtifactStub,
};
use orbit_tools::OrbitBuiltinAction;
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::input::optional_bool_alias;
use super::json::{learning_show_to_json, learning_to_json};

pub(super) fn add(
    runtime: &OrbitRuntime,
    input: Value,
    _agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let params = learning_create_params(&input, model)?;
    let learning = runtime.create_learning(params)?;
    Ok(learning_to_json(&learning))
}

impl OrbitRuntime {
    /// Complete one hub-issued learning ID in this exact runtime binding while
    /// preserving the public `orbit.learning.add` parser and result shape.
    pub fn finalize_preallocated_learning_tool(
        &self,
        expected_runtime_workspace_id: &str,
        id: &str,
        input: Value,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        self.verify_preallocated_owner_runtime(expected_runtime_workspace_id)?;
        let (input, redaction_report) =
            super::artifact_redaction::sanitize_tool_input(OrbitBuiltinAction::LearningAdd, input)?;
        let params = learning_create_params(&input, model.clone())?;
        let learning = self
            .stores()
            .learnings()
            .finalize_preallocated(id, params)?;
        let mut response = learning_to_json(&learning);
        super::artifact_redaction::finish_tool_response(
            self,
            OrbitBuiltinAction::LearningAdd,
            &mut response,
            &redaction_report,
            None,
            model.as_deref(),
        )?;
        Ok(response)
    }

    pub fn rollback_preallocated_learning(&self, id: &str) -> Result<bool, OrbitError> {
        self.stores().learnings().rollback_preallocated(id)
    }
}

fn learning_create_params(
    input: &Value,
    model: Option<String>,
) -> Result<LearningCreateParams, OrbitError> {
    let summary = required_string(input, &["summary"], "summary")?;
    let scope_value = input
        .get("scope")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let scope = parse_scope_value(scope_value)?;
    let body = optional_string(input, "body")?.unwrap_or_default();
    let evidence = parse_evidence_value(input.get("evidence"))?;
    let priority = parse_optional_priority(input)?;
    let created_by = optional_string_alias(input, &["created_by", "createdBy"])?.or(model);

    Ok(LearningCreateParams {
        summary,
        scope,
        body,
        evidence,
        created_by,
        priority,
    })
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let learning = runtime.get_learning(&id)?;
    // Opening the full body is the passive usage signal for this learning.
    runtime.record_learning_shown(&learning.id)?;
    Ok(learning_show_to_json(&learning))
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let status = optional_string(&input, "status")?
        .map(|raw| LearningStatus::from_str(&raw).map_err(OrbitError::InvalidInput))
        .transpose()?;
    let tag = optional_string(&input, "tag")?.map(|t| t.trim().to_lowercase());
    let path_normalized = optional_string(&input, "path")?
        .as_deref()
        .map(orbit_common::utility::glob::normalize_glob_path)
        .transpose()?;
    let include_remote =
        optional_bool_alias(&input, &["include_remote", "includeRemote"])?.unwrap_or(false);

    let learnings = runtime.list_learning_entries(status, include_remote)?;
    let filtered: Vec<_> = learnings
        .into_iter()
        .filter(|entry| {
            let LearningListEntry::Local(l) = entry else {
                return tag.is_none() && path_normalized.is_none();
            };
            if let Some(ref tag) = tag
                && !l.scope.tags.iter().any(|t| t == tag)
            {
                return false;
            }
            if let Some(ref path) = path_normalized
                && !l.scope.paths.iter().any(|rule| {
                    orbit_common::utility::glob::compile_glob_regex(rule)
                        .map(|regex| regex.is_match(path))
                        .unwrap_or(false)
                })
            {
                return false;
            }
            true
        })
        .collect();
    Ok(Value::Array(
        filtered.iter().map(learning_entry_to_json).collect(),
    ))
}

fn learning_entry_to_json(entry: &LearningListEntry) -> Value {
    match entry {
        LearningListEntry::Local(learning) => {
            let mut value = learning_to_json(learning);
            if let Some(object) = value.as_object_mut() {
                object.insert("remote".to_string(), Value::Bool(false));
            }
            value
        }
        LearningListEntry::Remote(stub) => remote_stub_to_json(stub),
    }
}

fn remote_stub_to_json(stub: &RemoteArtifactStub) -> Value {
    json!({
        "id": stub.id,
        "kind": stub.kind,
        "status": stub.status,
        "remote": true,
        "remote_marker": format!("[remote: {}]", stub.worktree_root.display()),
        "worktree_root": stub.worktree_root.to_string_lossy(),
        "branch": stub.branch,
        "body_path": stub.body_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "summary": Value::Null,
        "body": Value::Null,
        "scope": Value::Null,
        "evidence": Value::Null,
    })
}

pub(super) fn update(
    runtime: &OrbitRuntime,
    input: Value,
    _agent: Option<String>,
    _model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let summary = optional_string(&input, "summary")?;
    let scope = match input.get("scope") {
        Some(Value::Null) | None => None,
        Some(value) => Some(parse_scope_value(value.clone())?),
    };
    let body = optional_string(&input, "body")?;
    let evidence = match input.get("evidence") {
        Some(Value::Null) | None => None,
        Some(value) => Some(parse_evidence_value(Some(value))?),
    };
    let priority = parse_optional_priority_field(&input)?;

    let updated = runtime.stores().learnings().update(
        &id,
        LearningUpdateParams {
            summary,
            scope,
            body,
            evidence,
            priority,
        },
    )?;
    Ok(learning_to_json(&updated))
}

pub(super) fn supersede(
    runtime: &OrbitRuntime,
    input: Value,
    _agent: Option<String>,
    _model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id", "old_id", "oldId"], "id")?;
    let with = required_string(&input, &["with", "new_id", "newId"], "with")?;
    if id == with {
        return Err(OrbitError::InvalidInput(format!(
            "learning '{id}' cannot supersede itself"
        )));
    }
    runtime.stores().learnings().supersede(&id, &with)?;
    let old = runtime
        .stores()
        .learnings()
        .get(&id)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Learning, id.clone()))?;
    let new = runtime
        .stores()
        .learnings()
        .get(&with)?
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Learning, with.clone()))?;
    Ok(json!({
        "old": learning_to_json(&old),
        "new": learning_to_json(&new),
    }))
}

pub(super) fn sync(runtime: &OrbitRuntime, _input: Value) -> Result<Value, OrbitError> {
    runtime.stores().learnings().sync()?;
    let active = runtime
        .stores()
        .learnings()
        .list(Some(LearningStatus::Active))?;
    let superseded = runtime
        .stores()
        .learnings()
        .list(Some(LearningStatus::Superseded))?;
    Ok(json!({
        "rebuilt_count": active.len() + superseded.len(),
    }))
}

pub(super) fn prune(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    // Defaults: `stale_only = true` (report without modifying); `delete =
    // false`. `delete` flips stale → superseded with `superseded_by = null`
    // per ADR-004 / §7.3. The runtime owns the staleness logic so the CLI
    // (`orbit learning prune`) and MCP path produce identical results.
    let _stale_only = optional_bool_alias(&input, &["stale_only", "staleOnly"])?.unwrap_or(true);
    let delete = optional_bool_alias(&input, &["delete"])?.unwrap_or(false);
    let (stale, deleted) = runtime.prune_learnings(delete)?;
    Ok(json!({
        "stale": stale,
        "deleted": deleted,
    }))
}

fn parse_scope_value(value: Value) -> Result<LearningScope, OrbitError> {
    let mut scope = LearningScope::default();
    if value.is_null() {
        return Ok(scope);
    }
    let object = value
        .as_object()
        .ok_or_else(|| OrbitError::InvalidInput("`scope` must be a JSON object".to_string()))?;
    if let Some(paths) = object.get("paths") {
        scope.paths = parse_string_list("scope.paths", paths)?;
    }
    if let Some(tags) = object.get("tags") {
        scope.tags = parse_string_list("scope.tags", tags)?;
    }
    if let Some(symbols) = object.get("symbols") {
        scope.symbols = parse_string_list("scope.symbols", symbols)?;
    }
    if let Some(Value::String(seed)) = object.get("semantic_seed") {
        scope.semantic_seed = Some(seed.clone());
    }
    Ok(scope)
}

fn parse_string_list(field: &str, value: &Value) -> Result<Vec<String>, OrbitError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(raw) => Ok(vec![raw.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(format!("`{field}` entries must be strings"))
                    })
                    .map(str::to_string)
            })
            .collect(),
        _ => Err(OrbitError::InvalidInput(format!(
            "`{field}` must be a string or array of strings"
        ))),
    }
}

fn parse_evidence_value(value: Option<&Value>) -> Result<Vec<LearningEvidence>, OrbitError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items.iter().map(parse_evidence_item).collect(),
        _ => Err(OrbitError::InvalidInput(
            "`evidence` must be an array of `{kind, ref}` entries".to_string(),
        )),
    }
}

fn parse_evidence_item(value: &Value) -> Result<LearningEvidence, OrbitError> {
    let object = value.as_object().ok_or_else(|| {
        OrbitError::InvalidInput("`evidence` entries must be objects".to_string())
    })?;
    let kind_raw = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("`evidence.kind` must be a string".to_string()))?;
    let kind = EvidenceKind::from_str(kind_raw).map_err(OrbitError::InvalidInput)?;
    let reference = object
        .get("ref")
        .or_else(|| object.get("reference"))
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("`evidence.ref` must be a string".to_string()))?
        .to_string();
    Ok(LearningEvidence { kind, reference })
}

fn parse_optional_priority(input: &Value) -> Result<Option<u8>, OrbitError> {
    match input.get("priority") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => coerce_priority(value).map(Some),
    }
}

/// For update: `Some(Some(N))` sets, `Some(None)` clears, `None` keeps.
fn parse_optional_priority_field(input: &Value) -> Result<Option<Option<u8>>, OrbitError> {
    let object = match input.as_object() {
        Some(object) => object,
        None => return Ok(None),
    };
    if !object.contains_key("priority") {
        return Ok(None);
    }
    let value = &object["priority"];
    if value.is_null() {
        return Ok(Some(None));
    }
    coerce_priority(value).map(|n| Some(Some(n)))
}

fn coerce_priority(value: &Value) -> Result<u8, OrbitError> {
    let as_u64 = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| OrbitError::InvalidInput("`priority` must be an integer 0..=255".to_string()))?;
    u8::try_from(as_u64)
        .map_err(|_| OrbitError::InvalidInput("`priority` must be an integer 0..=255".to_string()))
}

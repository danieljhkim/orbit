use orbit_common::types::{
    AuditEventStatus, OrbitError, audit_execution_id, normalize_optional_attribution_label,
};
use orbit_common::utility::redaction::{
    is_high_confidence_credential_token, redact_all, redact_home_dir, redact_sensitive_env_text,
};
use serde_json::{Value, json};

use crate::{AuditEventInsertParams, OrbitRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactRedactionKind {
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
struct FieldRedaction {
    field: &'static str,
    kinds: Vec<ArtifactRedactionKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ArtifactRedactionReport {
    fields: Vec<FieldRedaction>,
}

impl ArtifactRedactionReport {
    pub(super) fn absorb(&mut self, field: SanitizedArtifactField) -> String {
        if !field.kinds.is_empty() {
            self.fields.push(FieldRedaction {
                field: field.field,
                kinds: field.kinds,
            });
        }
        field.value
    }

    pub(super) fn applied(&self) -> bool {
        !self.fields.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SanitizedArtifactField {
    field: &'static str,
    value: String,
    kinds: Vec<ArtifactRedactionKind>,
}

pub(super) fn sanitize_free_text_field(
    field: &'static str,
    input: String,
) -> Result<SanitizedArtifactField, OrbitError> {
    let env_scrubbed = redact_sensitive_env_text(&input);
    if env_scrubbed == input && is_high_confidence_credential_token(&input) {
        return Err(OrbitError::SensitiveInput(format!(
            "`{field}` is a complete credential token; remove it or embed only non-sensitive context"
        )));
    }

    let all_scrubbed = redact_all(&input);
    let home_scrubbed = redact_home_dir(&all_scrubbed);

    let mut kinds = Vec::new();
    if env_scrubbed != input {
        kinds.push(ArtifactRedactionKind::Env);
    }
    if all_scrubbed != env_scrubbed {
        kinds.push(ArtifactRedactionKind::Pattern);
    }
    if home_scrubbed != all_scrubbed {
        kinds.push(ArtifactRedactionKind::HomeDir);
    }

    Ok(SanitizedArtifactField {
        field,
        value: home_scrubbed,
        kinds,
    })
}

pub(super) fn sanitize_path_field(field: &'static str, input: String) -> SanitizedArtifactField {
    let home_scrubbed = redact_home_dir(&input);
    let kinds = if home_scrubbed != input {
        vec![ArtifactRedactionKind::HomeDir]
    } else {
        Vec::new()
    };
    SanitizedArtifactField {
        field,
        value: home_scrubbed,
        kinds,
    }
}

pub(super) fn redactions_flagged(mut value: Value, report: &ArtifactRedactionReport) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "redactions_applied".to_string(),
            Value::Bool(report.applied()),
        );
    }
    value
}

pub(super) fn flag_without_redactions(value: Value) -> Value {
    redactions_flagged(value, &ArtifactRedactionReport::default())
}

pub(super) fn emit_redaction_audits(
    runtime: &OrbitRuntime,
    tool_name: &str,
    artifact_type: &str,
    artifact_id: &str,
    report: &ArtifactRedactionReport,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<(), OrbitError> {
    let actor = normalize_optional_attribution_label(model.or(agent), model)
        .unwrap_or_else(|| runtime.actor_label().to_string());
    for field in &report.fields {
        let kinds = field
            .kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();
        let payload = json!({
            "artifact_type": artifact_type,
            "artifact_id": artifact_id,
            "field": field.field,
            "redaction_kinds": kinds,
            "actor": actor,
        });
        let arguments_json = serde_json::to_string(&payload).map_err(|error| {
            OrbitError::Execution(format!(
                "serialize artifact redaction audit payload: {error}"
            ))
        })?;

        runtime.record_audit_event(&AuditEventInsertParams {
            execution_id: audit_execution_id("audit-artifact-redaction"),
            command: "artifact".to_string(),
            subcommand: Some("redaction".to_string()),
            tool_name: Some(tool_name.to_string()),
            target_type: Some("artifact_redaction".to_string()),
            target_id: Some(artifact_id.to_string()),
            role: actor.clone(),
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
            task_id: None,
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

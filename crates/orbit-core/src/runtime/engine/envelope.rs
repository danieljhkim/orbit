use std::path::{Path, PathBuf};

use orbit_common::types::{
    Activity, AgentModelPair, OrbitError, Task, agent_family_from_cli, prune_missing_context_files,
};
use orbit_engine::{ExecutionContext, TaskHost};
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::{canonicalize_context_files_for_read, context_workspace_root};

#[derive(Debug, Clone, Serialize)]
struct ExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    activity: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    job: Option<Value>,
    skills: Vec<ExecutionSkillEnvelope>,
    input: Value,
    memory: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct ExecutionSkillEnvelope {
    id: String,
    content_hash: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

pub(super) fn build_agent_stdin_envelope_payload(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
) -> Result<Vec<u8>, OrbitError> {
    let skill_refs = activity_skill_refs_from_spec_config(&execution.activity.spec_config)?;
    let skills = runtime.resolve_activity_skill_refs(&skill_refs)?;
    fn activity_skill_refs_from_spec_config(
        spec_config: &JsonValue,
    ) -> Result<Vec<String>, OrbitError> {
        if !spec_config.is_object() {
            return Err(OrbitError::InvalidInput(
                "activity spec_config must be a JSON object".to_string(),
            ));
        }
        let Some(raw_refs) = spec_config.get("skill_refs") else {
            return Ok(Vec::new());
        };
        serde_json::from_value(raw_refs.clone()).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "activity spec_config.skill_refs must be an array of strings: {error}"
            ))
        })
    }
    let task = task_detail_for_input(
        runtime,
        &execution.input,
        &runtime.context.paths().repo_root,
    )?;
    let envelope = ExecutionEnvelope {
        schema_version: 1,
        activity: activity_envelope_json_for_execution_with_pair(
            &execution.activity,
            &execution.agent_cli,
            runtime.configured_agent_model_pair(&execution.agent_cli),
        ),
        job: execution.job.as_ref().map(|job| {
            json!({
                "id": job.job_id,
                "state": job.state,
                "default_input": job.default_input,
                "steps": job.steps.iter().map(|s| json!({
                    "target_type": s.target_type,
                    "target_id": s.target_id,
                    "agent_cli": s.agent_cli,
                    "model": s.model,
                    "timeout_seconds": s.timeout_seconds,
                })).collect::<Vec<_>>(),
            })
        }),
        skills: skills
            .into_iter()
            .map(|skill| ExecutionSkillEnvelope {
                id: skill.id,
                content_hash: skill.content_hash,
                content: skill.content,
                meta: skill.meta_raw,
            })
            .collect(),
        input: execution.input.clone(),
        memory: json!({}),
        task,
    };

    serde_json::to_vec(&envelope)
        .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
}

fn task_detail_for_input<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
    fallback_repo_root: &Path,
) -> Result<Option<Value>, OrbitError> {
    let Some(task_id) = input.get("task_id").and_then(Value::as_str) else {
        return Ok(None);
    };

    let task = host.get_task(task_id)?;
    Ok(Some(task_detail_envelope_json(
        &task,
        input,
        fallback_repo_root,
    )))
}

fn task_detail_envelope_json(task: &Task, input: &Value, fallback_repo_root: &Path) -> Value {
    let workspace_path = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let repo_root = input
        .get("repo_root")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    // Read-time safety net: drop any `context_files` entries whose resolved
    // paths no longer exist on disk. The authoritative fix lives at write-time
    // in orbit-core, but files can be deleted *after* a task is written, and
    // existing tasks on disk may still reference stale paths. Keep the on-disk
    // task untouched — this only filters what reaches the agent envelope.
    let prune_root: PathBuf = context_workspace_root(fallback_repo_root, workspace_path.as_deref());
    let canonical_context_files =
        canonicalize_context_files_for_read(&task.context_files, &prune_root);
    let (kept_context_files, _dropped) =
        prune_missing_context_files(&prune_root, canonical_context_files);

    json!({
        "id": task.id.clone(),
        "title": task.title.clone(),
        "description": task.description.clone(),
        "acceptance_criteria": task.acceptance_criteria.clone(),
        "plan": task.plan.clone(),
        "context_files": kept_context_files,
        "external_refs": task.external_refs.clone(),
        "workspace_path": workspace_path,
        "repo_root": repo_root,
    })
}

fn activity_envelope_json_for_execution_with_pair(
    activity: &Activity,
    agent_cli: &str,
    pair: Option<AgentModelPair>,
) -> Value {
    let family = agent_family_from_cli(agent_cli);
    let orchestrator = pair.as_ref().map(|p| p.orchestrator.as_str()).unwrap_or("");
    let helper = pair.as_ref().map(|p| p.helper.as_str()).unwrap_or("");

    let mut envelope = json!({
        "id": activity.id,
        "type": activity.spec_type,
        "description": activity.description,
        "input_schema_json": activity.input_schema_json,
        "created_by": activity.created_by,
    });

    if let Some(activity_map) = envelope.as_object_mut()
        && let Some(spec_config) = activity.spec_config.as_object()
    {
        for (key, value) in spec_config {
            activity_map.insert(key.clone(), value.clone());
        }
    }

    if let Some(activity_map) = envelope.as_object_mut() {
        activity_map.insert("agent_family".to_string(), json!(family));
        activity_map.insert("orchestrator_model".to_string(), json!(orchestrator));
        activity_map.insert("helper_model".to_string(), json!(helper));

        if let Some(instruction_value) = activity_map.get("instruction").cloned()
            && let Some(instruction_str) = instruction_value.as_str()
        {
            let rendered = render_agent_pair_placeholders(instruction_str, &family, &pair);
            activity_map.insert("instruction".to_string(), Value::String(rendered));
        }
    }

    envelope
}

fn render_agent_pair_placeholders(
    instruction: &str,
    family: &str,
    pair: &Option<AgentModelPair>,
) -> String {
    let family_value = if family.is_empty() {
        "(unspecified)".to_string()
    } else {
        family.to_string()
    };
    let (orchestrator_value, helper_value) = match pair {
        Some(pair) => (pair.orchestrator.clone(), pair.helper.clone()),
        None => (
            format!("(no orchestrator mapping for {family_value})"),
            format!("(no helper mapping for {family_value})"),
        ),
    };

    instruction
        .replace("{{agent_family}}", &family_value)
        .replace("{{orchestrator_model}}", &orchestrator_value)
        .replace("{{helper_model}}", &helper_value)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use orbit_common::types::Activity;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn agent_envelope_loads_planning_duel_skills_from_global_root() {
        let (_root, runtime, global_root, workspace_root) = test_runtime();
        write_skill(&global_root.join("skills"), "orbit", "global orbit skill");
        write_skill(
            &global_root.join("skills"),
            "orbit-graph",
            "global graph skill",
        );

        let envelope = build_test_envelope(&runtime, &["orbit", "orbit-graph"]);
        let skills = envelope_skills(&envelope);

        assert_eq!(skill_content(&skills, "orbit"), "global orbit skill");
        assert_eq!(skill_content(&skills, "orbit-graph"), "global graph skill");
        assert!(!workspace_root.join("skills").join("orbit").exists());
        assert!(
            !workspace_root
                .join("resources")
                .join("skills")
                .join("orbit")
                .join("SKILL.md")
                .exists()
        );
    }

    #[test]
    fn agent_envelope_preserves_workspace_skill_override_precedence() {
        let (_root, runtime, global_root, workspace_root) = test_runtime();
        write_skill(&global_root.join("skills"), "orbit", "global orbit skill");
        write_skill(
            &global_root.join("skills"),
            "orbit-graph",
            "global graph skill",
        );
        write_skill(
            &workspace_root.join("resources").join("skills"),
            "orbit",
            "workspace orbit override",
        );

        let envelope = build_test_envelope(&runtime, &["orbit", "orbit-graph"]);
        let skills = envelope_skills(&envelope);

        assert_eq!(skill_content(&skills, "orbit"), "workspace orbit override");
        assert_eq!(skill_content(&skills, "orbit-graph"), "global graph skill");
    }

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        fs::create_dir_all(&global_root).expect("create global root");
        fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
        (root, runtime, global_root, workspace_root)
    }

    fn build_test_envelope(runtime: &OrbitRuntime, skill_refs: &[&str]) -> Value {
        let execution = ExecutionContext {
            activity: test_activity(skill_refs),
            job: None,
            agent_cli: "codex".to_string(),
            model: Some("test-model".to_string()),
            timeout_seconds: 5,
            env_extra: Vec::new(),
            env_set: HashMap::new(),
            input: json!({}),
            debug: false,
            steps_outputs: HashMap::new(),
            run_id: None,
            step_index: None,
            state_dir: None,
        };
        let payload = build_agent_stdin_envelope_payload(runtime, &execution)
            .expect("build agent stdin envelope");
        serde_json::from_slice(&payload).expect("parse envelope json")
    }

    fn test_activity(skill_refs: &[&str]) -> Activity {
        let now = Utc::now();
        Activity {
            id: "propose_duel_plan".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "test planning duel activity".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Use the injected skills.",
                "skill_refs": skill_refs,
            }),
            tools: Vec::new(),
            proc_allowed_programs: Vec::new(),
            executor: None,
            workspace_path: None,
            created_by: Some("test".to_string()),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn envelope_skills(envelope: &Value) -> Vec<Value> {
        envelope
            .get("skills")
            .and_then(Value::as_array)
            .expect("skills array")
            .clone()
    }

    fn skill_content(skills: &[Value], id: &str) -> String {
        skills
            .iter()
            .find(|skill| skill.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|skill| skill.get("content").and_then(Value::as_str))
            .expect("skill content")
            .lines()
            .skip_while(|line| line.trim() != "# Purpose")
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

    fn write_skill(root: &Path, id: &str, purpose: &str) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("create skill dir");
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: test skill\n---\n\n# Purpose\n\n{purpose}\n"),
        )
        .expect("write skill");
    }
}

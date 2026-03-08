use std::collections::HashSet;

use orbit_policy::PolicyContext;
use orbit_types::{AgentToolCall, OrbitError, PolicyDecision, Role, Skill, Task};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::OrbitRuntime;

#[derive(Debug, Clone, Serialize)]
pub struct AgentComposedContext {
    pub instructions: String,
    pub context_files: Vec<String>,
    pub allowed_tools_raw: Option<Vec<String>>,
    pub effective_allowed_tools: Vec<String>,
    pub role: Role,
    pub skill_names: Vec<String>,
    pub composed_context_hash: String,
}

pub fn compose_agent_context(
    runtime: &OrbitRuntime,
    task: &Task,
    skills: &[Skill],
    runtime_role: Role,
    identity_block: Option<&str>,
) -> Result<AgentComposedContext, OrbitError> {
    let skill_names = skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let instructions = compose_instructions(task, skills, identity_block);
    let context_files = compose_context_files(task, skills);
    let allowed_tools_raw = compose_skill_allowlist(skills);
    let role = compose_role(runtime_role, skills);
    let policy_allowed_tools = policy_allowed_tools(runtime, role);
    let effective_allowed_tools = match &allowed_tools_raw {
        Some(raw) => intersect_preserving_left_order(raw, &policy_allowed_tools),
        None => policy_allowed_tools.clone(),
    };

    if effective_allowed_tools.is_empty() {
        return Err(OrbitError::PolicyDenied(
            "effective tool allowlist is empty after policy and skill intersection".to_string(),
        ));
    }

    let mut context = AgentComposedContext {
        instructions,
        context_files,
        allowed_tools_raw,
        effective_allowed_tools,
        role,
        skill_names,
        composed_context_hash: String::new(),
    };
    context.composed_context_hash = hash_context(&context)?;
    Ok(context)
}

pub fn parse_planned_tool_calls(plan_payload: &str) -> Result<Vec<AgentToolCall>, OrbitError> {
    if plan_payload.trim().is_empty() {
        return Err(OrbitError::AgentRun(
            "task plan is empty; expected JSON payload".to_string(),
        ));
    }

    let payload: Value = serde_json::from_str(plan_payload)
        .map_err(|e| OrbitError::AgentRun(format!("invalid task plan JSON: {e}")))?;

    let call_items = match payload {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                OrbitError::AgentRun(
                    "plan payload must be an array or object with `tool_calls` array".to_string(),
                )
            })?,
        _ => {
            return Err(OrbitError::AgentRun(
                "plan payload must be an array or object".to_string(),
            ));
        }
    };

    if call_items.is_empty() {
        return Err(OrbitError::AgentRun(
            "plan payload contains no tool calls".to_string(),
        ));
    }

    let mut calls = Vec::with_capacity(call_items.len());
    for (idx, call) in call_items.into_iter().enumerate() {
        let obj = call.as_object().ok_or_else(|| {
            OrbitError::AgentRun(format!("tool call at index {idx} must be an object"))
        })?;
        let name = obj.get("name").and_then(Value::as_str).ok_or_else(|| {
            OrbitError::AgentRun(format!("tool call at index {idx} missing `name`"))
        })?;
        let input = obj.get("input").cloned().unwrap_or_else(|| json!({}));
        calls.push(AgentToolCall {
            name: name.to_string(),
            input,
            output: None,
            success: false,
        });
    }

    Ok(calls)
}

fn compose_instructions(task: &Task, skills: &[Skill], identity_block: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(block) = identity_block
        && !block.trim().is_empty()
    {
        parts.push(block.trim().to_string());
    }
    if !task.description.trim().is_empty() {
        parts.push(task.description.trim().to_string());
    }
    if !task.plan.trim().is_empty() {
        parts.push(task.plan.trim().to_string());
    }
    for skill in skills {
        if !skill.instructions.trim().is_empty() {
            parts.push(skill.instructions.trim().to_string());
        }
    }
    parts.join("\n\n")
}

fn compose_context_files(task: &Task, skills: &[Skill]) -> Vec<String> {
    let mut values = Vec::new();
    values.extend(task.context_files.iter().cloned());
    for skill in skills {
        values.extend(skill.context_files.iter().cloned());
    }
    dedup_keep_first(values)
}

fn compose_skill_allowlist(skills: &[Skill]) -> Option<Vec<String>> {
    let mut declared = skills
        .iter()
        .filter(|s| !s.allowed_tools.is_empty())
        .map(|s| dedup_keep_first(s.allowed_tools.clone()));

    let first = declared.next()?;
    let mut intersection = first;
    for tools in declared {
        let set = tools.into_iter().collect::<HashSet<_>>();
        intersection.retain(|tool| set.contains(tool));
    }
    Some(intersection)
}

fn compose_role(runtime_role: Role, skills: &[Skill]) -> Role {
    skills.iter().fold(runtime_role, |acc, skill| {
        most_restrictive_role(acc, skill.role)
    })
}

fn most_restrictive_role(left: Role, right: Role) -> Role {
    match (left, right) {
        (Role::Agent, _) | (_, Role::Agent) => Role::Agent,
        _ => Role::Admin,
    }
}

fn policy_allowed_tools(runtime: &OrbitRuntime, role: Role) -> Vec<String> {
    let mut tool_names = runtime
        .context
        .registry
        .schemas()
        .into_iter()
        .map(|s| s.name)
        .collect::<Vec<_>>();
    tool_names.sort();

    tool_names
        .into_iter()
        .filter(|name| {
            matches!(
                runtime.context.policy.evaluate(&PolicyContext {
                    entrypoint: "agent".to_string(),
                    tool_name: Some(name.clone()),
                    role,
                }),
                PolicyDecision::Allow
            )
        })
        .collect()
}

fn intersect_preserving_left_order(left: &[String], right: &[String]) -> Vec<String> {
    let right_set = right.iter().cloned().collect::<HashSet<_>>();
    left.iter()
        .filter(|value| right_set.contains(value.as_str()))
        .cloned()
        .collect()
}

fn dedup_keep_first(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn hash_context(context: &AgentComposedContext) -> Result<String, OrbitError> {
    let bytes = serde_json::to_vec(context)
        .map_err(|e| OrbitError::AgentRun(format!("failed to serialize composed context: {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    Ok(output)
}

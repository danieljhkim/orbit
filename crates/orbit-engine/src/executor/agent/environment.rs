use orbit_agent::Agent;
use orbit_exec::EnvironmentMode;
use orbit_types::OrbitError;

use crate::context::EnvironmentHost;

use super::execution::ResolvedAgentExecution;

pub(crate) fn inject_activity_tools(mode: EnvironmentMode, tools: &[String]) -> EnvironmentMode {
    inject_csv_env(mode, "ORBIT_ACTIVITY_TOOLS", tools)
}

pub(crate) fn inject_proc_allowed_programs(
    mode: EnvironmentMode,
    programs: &[String],
) -> EnvironmentMode {
    inject_csv_env(mode, "ORBIT_PROC_ALLOWED_PROGRAMS", programs)
}

pub(crate) fn inject_agent_identity(
    mode: EnvironmentMode,
    agent_label: &str,
    resolved_model: Option<&str>,
) -> EnvironmentMode {
    let agent = normalize_agent_label(agent_label);
    if agent.is_empty() {
        return mode;
    }

    inject_environment(mode, |pairs| {
        pairs.push(("ORBIT_AGENT_NAME".to_string(), agent.clone()));
        if let Some(model) = resolved_model.filter(|value| !value.is_empty()) {
            pairs.push(("ORBIT_AGENT_MODEL".to_string(), model.to_string()));
        }
    })
}

pub(super) fn resolve_model_for_env<H: EnvironmentHost + ?Sized>(
    host: &H,
    resolved: &ResolvedAgentExecution,
) -> Option<String> {
    let config = host
        .agent_config_for(&resolved.command, resolved.model.as_deref())
        .ok()?;
    let model_from_config = config.model.clone().or_else(|| resolved.model.clone());
    let agent = Agent::new(&config).ok();
    agent
        .and_then(|agent| agent.model_name().map(ToOwned::to_owned))
        .or(model_from_config)
}

fn inject_csv_env(mode: EnvironmentMode, key: &str, values: &[String]) -> EnvironmentMode {
    if values.is_empty() {
        return mode;
    }

    let joined = values.join(",");
    inject_environment(mode, |pairs| pairs.push((key.to_string(), joined.clone())))
}

fn inject_environment<F>(mode: EnvironmentMode, inject: F) -> EnvironmentMode
where
    F: FnOnce(&mut Vec<(String, String)>),
{
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            inject(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            inject(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

#[allow(dead_code)]
fn _type_check(_: OrbitError) {}

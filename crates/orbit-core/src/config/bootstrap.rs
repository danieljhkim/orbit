use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::OrbitError;
use serde::Serialize;

use orbit_common::utility::fs::write_text_with_parent;

use super::agent_detect::{
    DetectedAgents, available_crew_families, default_crew_name, default_model_for,
};
use super::raw::{RawAgentRoleConfig, RawCrewEntry, RawDuelSection};
use super::runtime::default_crews;

pub(crate) const DEFAULT_CONFIG_TEMPLATE: &str =
    include_str!("../../assets/config/default-config.toml"); // pub(crate) for sibling tests/bootstrap.rs per ORB-00223; no prod behavior change.

pub(crate) fn seed_default_config(
    config_path: &Path,
    detected: &DetectedAgents,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<bool, OrbitError> {
    if config_path.exists() {
        return Ok(false);
    }
    let body = render_seeded_config(DEFAULT_CONFIG_TEMPLATE, detected, role_settings)?;
    write_text_with_parent(config_path, &body)?;
    Ok(true)
}

fn render_seeded_config(
    template: &str,
    detected: &DetectedAgents,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<String, OrbitError> {
    let role_settings = role_settings.filter(|roles| !roles.is_empty());
    if let Some(roles) = role_settings {
        validate_complete_role_settings(roles)?;
    }

    let mut body = template.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }

    // ADR-0193: freeze agent detection at init; runtime config loading never probes PATH/env.
    body.push_str(&render_workflow_default_crew(detected, role_settings));
    body.push('\n');
    body.push_str(&render_crews(detected, role_settings)?);
    body.push_str(&render_duel(detected)?);
    Ok(body)
}

fn render_workflow_default_crew(
    detected: &DetectedAgents,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> String {
    let default_crew = if role_settings.is_some() {
        "custom"
    } else {
        default_crew_name(detected)
    };
    format!("default_crew = \"{default_crew}\"\n")
}

fn render_crews(
    detected: &DetectedAgents,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<String, OrbitError> {
    let mut crews: BTreeMap<String, RawCrewEntry> = default_crews()
        .into_iter()
        .map(|(name, crew)| {
            (
                name,
                RawCrewEntry {
                    provider: Some(crew.assignment.provider),
                    model: Some(crew.assignment.model),
                    backend: Some(crew.assignment.backend),
                    planner: None,
                    implementer: None,
                    reviewer: None,
                },
            )
        })
        .collect();

    if let Some(roles) = role_settings {
        let assignment = roles.get("implementer").ok_or_else(|| {
            OrbitError::InvalidInput(
                "custom crew is missing required `implementer` settings".to_string(),
            )
        })?;
        crews.insert(
            "custom".to_string(),
            RawCrewEntry {
                provider: assignment.provider.clone(),
                model: assignment.model.clone(),
                backend: assignment.backend.clone(),
                planner: None,
                implementer: None,
                reviewer: None,
            },
        );
    }

    let qa = role_settings
        .and_then(|roles| roles.get("qa").cloned())
        .unwrap_or_else(|| default_qa_crew(detected));
    crews.insert(
        "qa".to_string(),
        RawCrewEntry {
            provider: qa.provider,
            model: qa.model,
            backend: qa.backend,
            planner: None,
            implementer: None,
            reviewer: None,
        },
    );

    let mut rendered = String::new();
    for (name, entry) in crews {
        rendered.push_str(&render_crew_table(&name, &entry)?);
    }
    Ok(rendered)
}

fn default_qa_crew(detected: &DetectedAgents) -> RawAgentRoleConfig {
    let (provider, backend, model) = if detected.codex_cli || detected.openai_api_key {
        (
            "codex",
            super::agent_detect::default_backend("codex", detected),
            orbit_common::model_defaults::CODEX_DEFAULT_MODEL,
        )
    } else if detected.claude_cli || detected.anthropic_api_key {
        (
            "claude",
            super::agent_detect::default_backend("claude", detected),
            orbit_common::model_defaults::CLAUDE_DEFAULT_WEAK,
        )
    } else {
        (
            "codex",
            "cli",
            orbit_common::model_defaults::CODEX_DEFAULT_MODEL,
        )
    };
    RawAgentRoleConfig {
        provider: Some(provider.to_string()),
        backend: Some(backend.to_string()),
        model: Some(model.to_string()),
    }
}

fn render_crew_table(name: &str, entry: &RawCrewEntry) -> Result<String, OrbitError> {
    let mut rendered = format!("[crews.{name}]\n");
    for (field, value) in [
        ("model", entry.model.as_deref()),
        ("provider", entry.provider.as_deref()),
        ("backend", entry.backend.as_deref()),
    ] {
        let value = value.ok_or_else(|| {
            OrbitError::InvalidInput(format!("crew `{name}` is missing `{field}`"))
        })?;
        rendered.push_str(&format!(
            "{field} = {}\n",
            toml::Value::String(value.to_string())
        ));
    }
    rendered.push('\n');
    Ok(rendered)
}

fn render_duel(detected: &DetectedAgents) -> Result<String, OrbitError> {
    let candidates = available_crew_families(detected);
    if candidates.len() < 3 {
        return Ok(String::new());
    }

    #[derive(Serialize)]
    struct DuelConfig {
        duel: RawDuelSection,
    }

    let mut models = BTreeMap::new();
    for family in &candidates {
        let model = default_model_for(family).ok_or_else(|| {
            OrbitError::InvalidInput(format!("no default model configured for `{family}`"))
        })?;
        models.insert((*family).to_string(), model.to_string());
    }

    let mut rendered = toml::to_string(&DuelConfig {
        duel: RawDuelSection {
            candidates: Some(candidates.into_iter().map(str::to_string).collect()),
            models: Some(models),
        },
    })
    .map_err(|err| OrbitError::Io(format!("serialize [duel] sections: {err}")))?;
    if !rendered.starts_with('\n') {
        rendered.insert(0, '\n');
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn validate_complete_role_settings(
    roles: &BTreeMap<String, RawAgentRoleConfig>,
) -> Result<(), OrbitError> {
    let config = roles.get("implementer").ok_or_else(|| {
        OrbitError::InvalidInput(
            "custom crew is missing required `implementer` settings".to_string(),
        )
    })?;
    for (field, value) in [
        ("provider", config.provider.as_deref()),
        ("backend", config.backend.as_deref()),
        ("model", config.model.as_deref()),
    ] {
        if value.map(str::trim).is_none_or(str::is_empty) {
            return Err(OrbitError::InvalidInput(format!(
                "custom crew is missing required `{field}`"
            )));
        }
    }
    Ok(())
}

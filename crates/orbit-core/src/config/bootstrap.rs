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
    body.push_str(&render_crews(role_settings)?);
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
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<String, OrbitError> {
    let mut crews: BTreeMap<String, RawCrewEntry> = default_crews()
        .into_iter()
        .map(|(name, crew)| {
            (
                name,
                RawCrewEntry {
                    planner: Some(raw_role_from_assignment(&crew.planner)),
                    implementer: Some(raw_role_from_assignment(&crew.implementer)),
                    reviewer: Some(raw_role_from_assignment(&crew.reviewer)),
                },
            )
        })
        .collect();

    if let Some(roles) = role_settings {
        crews.insert(
            "custom".to_string(),
            RawCrewEntry {
                planner: roles.get("planner").cloned(),
                implementer: roles.get("implementer").cloned(),
                reviewer: roles.get("reviewer").cloned(),
            },
        );
    }

    let mut rendered = String::new();
    for (name, entry) in crews {
        rendered.push_str(&render_crew_table(&name, &entry)?);
    }
    Ok(rendered)
}

fn render_crew_table(name: &str, entry: &RawCrewEntry) -> Result<String, OrbitError> {
    let mut rendered = format!("[crews.{name}]\n");
    for (role, config) in [
        ("planner", entry.planner.as_ref()),
        ("implementer", entry.implementer.as_ref()),
        ("reviewer", entry.reviewer.as_ref()),
    ] {
        let config = config.ok_or_else(|| {
            OrbitError::InvalidInput(format!("crew `{name}` is missing `{role}` role settings"))
        })?;
        let value = toml::Value::try_from(config).map_err(|err| {
            OrbitError::Io(format!("serialize [crews.{name}].{role} assignment: {err}"))
        })?;
        rendered.push_str(&format!("{role} = {value}\n"));
    }
    rendered.push('\n');
    Ok(rendered)
}

fn raw_role_from_assignment(
    assignment: &orbit_common::types::CrewRoleAssignment,
) -> RawAgentRoleConfig {
    RawAgentRoleConfig {
        provider: Some(assignment.provider.clone()),
        model: Some(assignment.model.clone()),
        backend: Some(assignment.backend.clone()),
    }
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
    for role in ["planner", "implementer", "reviewer"] {
        let Some(config) = roles.get(role) else {
            return Err(OrbitError::InvalidInput(format!(
                "custom crew is missing required `{role}` role settings"
            )));
        };
        for (field, value) in [
            ("provider", config.provider.as_deref()),
            ("backend", config.backend.as_deref()),
            ("model", config.model.as_deref()),
        ] {
            if value.map(str::trim).is_none_or(str::is_empty) {
                return Err(OrbitError::InvalidInput(format!(
                    "custom crew role `{role}` is missing required `{field}`"
                )));
            }
        }
    }
    Ok(())
}

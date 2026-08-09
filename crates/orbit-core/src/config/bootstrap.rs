use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::OrbitError;

use orbit_common::utility::fs::write_text_with_parent;

use super::agent_detect::{DetectedAgents, available_crew_families, default_crew_name};
use super::raw::{RawCrewAssignment, RawCrewEntry};
use super::runtime::default_crews;

pub(crate) const DEFAULT_CONFIG_TEMPLATE: &str =
    include_str!("../../assets/config/default-config.toml"); // pub(crate) for sibling tests/bootstrap.rs per ORB-00223; no prod behavior change.

pub(crate) fn seed_default_config(
    config_path: &Path,
    detected: &DetectedAgents,
    crew_settings: Option<&BTreeMap<String, RawCrewAssignment>>,
) -> Result<bool, OrbitError> {
    if config_path.exists() {
        return Ok(false);
    }
    let body = render_seeded_config(DEFAULT_CONFIG_TEMPLATE, detected, crew_settings)?;
    write_text_with_parent(config_path, &body)?;
    Ok(true)
}

fn render_seeded_config(
    template: &str,
    detected: &DetectedAgents,
    crew_settings: Option<&BTreeMap<String, RawCrewAssignment>>,
) -> Result<String, OrbitError> {
    let crew_settings = crew_settings.filter(|settings| !settings.is_empty());
    if let Some(custom) = crew_settings.and_then(|settings| settings.get("custom")) {
        validate_complete_crew_setting(custom)?;
    }

    let mut body = template.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }

    // ADR-0193: freeze agent detection at init; runtime config loading never probes PATH/env.
    let workflow_default = render_workflow_default_crew(detected, crew_settings);
    if !workflow_default.is_empty() {
        // L-0100: generated TOML keys must be inserted inside their intended table.
        let marker = "[workflow]\n";
        let insertion = body.find(marker).ok_or_else(|| {
            OrbitError::InvalidInput("default config template is missing [workflow]".to_string())
        })? + marker.len();
        body.insert_str(insertion, &workflow_default);
    }
    body.push('\n');
    body.push_str(&render_crews(detected, crew_settings)?);
    Ok(body)
}

fn render_workflow_default_crew(
    detected: &DetectedAgents,
    crew_settings: Option<&BTreeMap<String, RawCrewAssignment>>,
) -> String {
    let default_crew = if crew_settings.is_some_and(|settings| settings.contains_key("custom")) {
        Some("custom")
    } else {
        default_crew_name(detected)
    };
    default_crew.map_or_else(String::new, |name| format!("default_crew = \"{name}\"\n"))
}

fn render_crews(
    detected: &DetectedAgents,
    crew_settings: Option<&BTreeMap<String, RawCrewAssignment>>,
) -> Result<String, OrbitError> {
    let available_families = available_crew_families(detected);
    let mut crews: BTreeMap<String, RawCrewEntry> = default_crews()
        .into_iter()
        .filter(|(_, crew)| available_families.contains(&crew.assignment.provider.as_str()))
        .map(|(name, crew)| {
            (
                name,
                RawCrewEntry {
                    provider: Some(crew.assignment.provider),
                    model: Some(crew.assignment.model),
                    backend: Some(crew.assignment.backend),
                    description: crew.description,
                    tags: crew.tags,
                    planner: None,
                    implementer: None,
                    reviewer: None,
                },
            )
        })
        .collect();

    if let Some(assignment) = crew_settings.and_then(|settings| settings.get("custom")) {
        crews.insert(
            "custom".to_string(),
            RawCrewEntry {
                provider: assignment.provider.clone(),
                model: assignment.model.clone(),
                backend: assignment.backend.clone(),
                description: None,
                tags: Vec::new(),
                planner: None,
                implementer: None,
                reviewer: None,
            },
        );
    }

    if let Some(qa) = crew_settings
        .and_then(|settings| settings.get("qa").cloned())
        .or_else(|| default_qa_crew(detected))
    {
        crews.insert(
            "qa".to_string(),
            RawCrewEntry {
                provider: qa.provider,
                model: qa.model,
                backend: qa.backend,
                description: None,
                tags: Vec::new(),
                planner: None,
                implementer: None,
                reviewer: None,
            },
        );
    }

    let mut rendered = String::new();
    for (name, entry) in crews {
        rendered.push_str(&render_crew_table(&name, &entry)?);
    }
    if rendered.is_empty() {
        // Preserve an explicitly empty registry so runtime loading does not
        // substitute built-in crews for a host where init detected none.
        rendered.push_str("[crews]\n");
    }
    Ok(rendered)
}

fn default_qa_crew(detected: &DetectedAgents) -> Option<RawCrewAssignment> {
    let (provider, model) = if detected.codex_cli {
        ("codex", orbit_common::model_defaults::CODEX_DEFAULT_MODEL)
    } else if detected.claude_cli {
        ("claude", orbit_common::model_defaults::CLAUDE_DEFAULT_WEAK)
    } else {
        return None;
    };
    Some(RawCrewAssignment {
        provider: Some(provider.to_string()),
        backend: Some("cli".to_string()),
        model: Some(model.to_string()),
    })
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
    if let Some(description) = entry
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push_str(&format!(
            "description = {}\n",
            toml::Value::String(description.to_string())
        ));
    }
    if !entry.tags.is_empty() {
        let tags = entry
            .tags
            .iter()
            .map(|tag| toml::Value::String(tag.clone()))
            .collect::<Vec<_>>();
        rendered.push_str(&format!("tags = {}\n", toml::Value::Array(tags)));
    }
    rendered.push('\n');
    Ok(rendered)
}

fn validate_complete_crew_setting(config: &RawCrewAssignment) -> Result<(), OrbitError> {
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

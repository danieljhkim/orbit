use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::OrbitError;
use serde::Serialize;

use orbit_common::utility::fs::write_text_with_parent;

use super::raw::{RawAgentRoleConfig, RawCrewEntry};

pub(crate) const DEFAULT_CONFIG_TEMPLATE: &str =
    include_str!("../../assets/config/default-config.toml"); // pub(crate) for sibling tests/bootstrap.rs per ORB-00223; no prod behavior change.

pub(crate) fn seed_default_config(
    config_path: &Path,
    role_settings: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<bool, OrbitError> {
    if config_path.exists() {
        return Ok(false);
    }
    let body = match role_settings.filter(|m| !m.is_empty()) {
        Some(roles) => render_with_role_settings(DEFAULT_CONFIG_TEMPLATE, roles)?,
        None => DEFAULT_CONFIG_TEMPLATE.to_string(),
    };
    write_text_with_parent(config_path, &body)?;
    Ok(true)
}

fn render_with_role_settings(
    template: &str,
    roles: &BTreeMap<String, RawAgentRoleConfig>,
) -> Result<String, OrbitError> {
    validate_complete_role_settings(roles)?;

    #[derive(Serialize)]
    struct CrewConfig<'a> {
        crews: BTreeMap<&'a str, RawCrewEntry>,
    }

    let mut crews = BTreeMap::new();
    crews.insert(
        "custom",
        RawCrewEntry {
            planner: roles.get("planner").cloned(),
            implementer: roles.get("implementer").cloned(),
            reviewer: roles.get("reviewer").cloned(),
        },
    );
    let custom_crew = toml::to_string(&CrewConfig { crews })
        .map_err(|err| OrbitError::Io(format!("serialize [crews.<name>] sections: {err}")))?;
    let mut body = template.replace("default_crew = \"opus-codex\"", "default_crew = \"custom\"");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push('\n');
    body.push_str(&custom_crew);
    Ok(body)
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

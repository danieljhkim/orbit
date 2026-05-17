use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::OrbitError;
use serde::Serialize;

use orbit_common::utility::fs::write_text_with_parent;

use super::raw::{RawAgentRoleConfig, RawCrewEntry};

const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../../assets/config/default-config.toml");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::raw::RawAgentRoleConfig;
    use tempfile::tempdir;

    fn sample_roles() -> BTreeMap<String, RawAgentRoleConfig> {
        let mut roles = BTreeMap::new();
        roles.insert(
            "reviewer".to_string(),
            RawAgentRoleConfig {
                provider: Some("claude".into()),
                backend: Some("cli".into()),
                model: Some("claude-opus-4-7".into()),
            },
        );
        roles.insert(
            "implementer".to_string(),
            RawAgentRoleConfig {
                provider: Some("codex".into()),
                backend: Some("cli".into()),
                model: Some("gpt-5.5".into()),
            },
        );
        roles.insert(
            "planner".to_string(),
            RawAgentRoleConfig {
                provider: Some("claude".into()),
                backend: Some("http".into()),
                model: Some("claude-opus-4-7".into()),
            },
        );
        roles
    }

    #[test]
    fn seed_with_no_role_settings_matches_template() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let created = seed_default_config(&path, None).expect("seed");
        assert!(created);
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents, DEFAULT_CONFIG_TEMPLATE);
        assert!(no_active_role_section(&contents));
        assert!(contents.contains("[crews.opus-codex]"));
        assert!(contents.contains("[crews.all-claude]"));
        assert!(contents.contains("default_crew = \"opus-codex\""));
    }

    fn no_active_role_section(contents: &str) -> bool {
        contents
            .lines()
            .all(|line| !line.trim_start().starts_with("[agent."))
    }

    #[test]
    fn seed_with_role_settings_writes_custom_crew() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let roles = sample_roles();
        let created = seed_default_config(&path, Some(&roles)).expect("seed");
        assert!(created);
        let contents = std::fs::read_to_string(&path).expect("read");

        assert!(no_active_role_section(&contents));
        assert!(contents.contains("default_crew = \"custom\""));
        assert!(contents.contains("provider = \"claude\""));
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains("model = \"claude-opus-4-7\""));
        assert!(contents.contains("model = \"gpt-5.5\""));

        // Round-trips through toml::from_str (consumer side will need this).
        let parsed: toml::Value = toml::from_str(&contents).expect("parse");
        let crews = parsed
            .get("crews")
            .expect("crews table")
            .as_table()
            .unwrap();
        let custom = crews
            .get("custom")
            .and_then(|v| v.as_table())
            .expect("custom crew");
        assert!(custom.contains_key("reviewer"));
        assert!(custom.contains_key("implementer"));
        assert!(custom.contains_key("planner"));
    }

    #[test]
    fn seed_with_existing_file_is_noop() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# pre-existing user content\n").expect("preseed");

        let roles = sample_roles();
        let created = seed_default_config(&path, Some(&roles)).expect("seed");
        assert!(!created);

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents, "# pre-existing user content\n");
    }

    #[test]
    fn seed_with_empty_role_map_matches_template() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let roles: BTreeMap<String, RawAgentRoleConfig> = BTreeMap::new();
        let created = seed_default_config(&path, Some(&roles)).expect("seed");
        assert!(created);
        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents, DEFAULT_CONFIG_TEMPLATE);
    }

    #[test]
    fn seed_with_incomplete_role_settings_fails() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut roles = sample_roles();
        roles.get_mut("planner").expect("planner").model.take();
        let error = seed_default_config(&path, Some(&roles)).expect_err("missing model fails");
        assert!(
            error
                .to_string()
                .contains("custom crew role `planner` is missing required `model`")
        );
        assert!(!path.exists());
    }
}

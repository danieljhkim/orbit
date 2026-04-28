use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::OrbitError;
use serde::Serialize;

use orbit_common::utility::fs::write_text_with_parent;

use super::raw::RawAgentRoleConfig;

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
    #[derive(Serialize)]
    struct AgentSection<'a> {
        agent: &'a BTreeMap<String, RawAgentRoleConfig>,
    }
    let serialized = toml::to_string(&AgentSection { agent: roles })
        .map_err(|err| OrbitError::Io(format!("serialize [agent.<role>] sections: {err}")))?;

    let mut out = String::with_capacity(template.len() + serialized.len() + 2);
    out.push_str(template);
    if !template.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&serialized);
    Ok(out)
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
                model: None,
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
        assert!(no_active_agent_section(&contents));
    }

    /// Returns true when the file has no uncommented `[agent.<role>]`
    /// section header. The default template ships with a commented-out
    /// documentation block that includes `# [agent.reviewer]` etc; we want
    /// to ignore those.
    fn no_active_agent_section(contents: &str) -> bool {
        contents
            .lines()
            .all(|line| !line.trim_start().starts_with("[agent."))
    }

    #[test]
    fn seed_with_role_settings_appends_agent_blocks() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let roles = sample_roles();
        let created = seed_default_config(&path, Some(&roles)).expect("seed");
        assert!(created);
        let contents = std::fs::read_to_string(&path).expect("read");

        // Default template is preserved verbatim at the head of the file.
        assert!(contents.starts_with(DEFAULT_CONFIG_TEMPLATE));

        // All three role tables are present.
        assert!(contents.contains("[agent.reviewer]"));
        assert!(contents.contains("[agent.implementer]"));
        assert!(contents.contains("[agent.planner]"));
        assert!(contents.contains("provider = \"claude\""));
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains("model = \"claude-opus-4-7\""));
        assert!(contents.contains("model = \"gpt-5.5\""));

        // Round-trips through toml::from_str (consumer side will need this).
        let parsed: toml::Value = toml::from_str(&contents).expect("parse");
        let agent = parsed
            .get("agent")
            .expect("agent table")
            .as_table()
            .unwrap();
        assert!(agent.contains_key("reviewer"));
        assert!(agent.contains_key("implementer"));
        assert!(agent.contains_key("planner"));
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
    fn seed_serialization_omits_none_fields() {
        // The planner sample omits `model`. Verify it doesn't show up as
        // `model = ""` or similar in the parsed output.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let roles = sample_roles();
        seed_default_config(&path, Some(&roles)).expect("seed");
        let contents = std::fs::read_to_string(&path).expect("read");

        // Parsing TOML ignores comments, so the only `[agent.planner]` table
        // visible in the parsed structure is the one we wrote.
        let parsed: toml::Value = toml::from_str(&contents).expect("parse");
        let planner = parsed
            .get("agent")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("planner"))
            .and_then(|v| v.as_table())
            .expect("planner table present");
        assert!(
            planner.get("model").is_none(),
            "planner.model must be absent when None: {planner:?}"
        );
        assert_eq!(
            planner.get("provider").and_then(|v| v.as_str()),
            Some("claude")
        );
        assert_eq!(
            planner.get("backend").and_then(|v| v.as_str()),
            Some("http")
        );
    }
}

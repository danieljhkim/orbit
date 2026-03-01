use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use orbit_types::OrbitError;
use serde::Deserialize;

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_ENV_PASS: [&str; 5] = [
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "HOME",
    "PATH",
    "CODEX_HOME",
];

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            execution_env: ExecutionEnvPolicy::default(),
        }
    }
}

impl RuntimeConfig {
    pub(crate) fn load_from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        let config_path = data_root.join("config.toml");
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&config_path).map_err(|err| {
            OrbitError::Io(format!(
                "failed to read runtime config '{}': {err}",
                config_path.display()
            ))
        })?;
        let parsed = toml::from_str::<RawRuntimeConfig>(&raw).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid runtime config '{}': {err}",
                config_path.display()
            ))
        })?;

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_raw(parsed.execution.and_then(|v| v.env))?,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionEnvPolicy {
    inherit: bool,
    pass: Vec<String>,
}

impl Default for ExecutionEnvPolicy {
    fn default() -> Self {
        Self {
            inherit: DEFAULT_ENV_INHERIT,
            pass: default_pass_list(),
        }
    }
}

impl ExecutionEnvPolicy {
    fn from_raw(raw: Option<RawExecutionEnvConfig>) -> Result<Self, OrbitError> {
        match raw {
            Some(raw) => {
                let inherit = raw.inherit.unwrap_or(DEFAULT_ENV_INHERIT);
                let pass = normalize_pass_list(raw.pass.unwrap_or_else(default_pass_list))?;
                Ok(Self { inherit, pass })
            }
            None => Ok(Self::default()),
        }
    }

    pub(crate) fn inherit(&self) -> bool {
        self.inherit
    }

    pub(crate) fn pass(&self) -> &[String] {
        &self.pass
    }

    pub(crate) fn hydrated_allowlist_env(&self) -> Vec<(String, String)> {
        self.pass
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect()
    }

    pub(crate) fn missing_required_for_provider(&self, provider: &str) -> Vec<String> {
        required_env_vars_for_provider(provider)
            .into_iter()
            .filter(|name| !self.is_required_var_available(name))
            .map(ToString::to_string)
            .collect()
    }

    fn is_required_var_available(&self, name: &str) -> bool {
        if self.inherit {
            return std::env::var(name).is_ok();
        }
        self.pass.iter().any(|candidate| candidate == name) && std::env::var(name).is_ok()
    }
}

fn default_pass_list() -> Vec<String> {
    DEFAULT_ENV_PASS.iter().map(ToString::to_string).collect()
}

fn normalize_pass_list(pass: Vec<String>) -> Result<Vec<String>, OrbitError> {
    let mut normalized = BTreeSet::new();
    for entry in pass {
        let value = entry.trim();
        if value.is_empty() {
            return Err(OrbitError::InvalidInput(
                "execution.env.pass must not contain empty variable names".to_string(),
            ));
        }
        if !is_valid_env_var_name(value) {
            return Err(OrbitError::InvalidInput(format!(
                "execution.env.pass contains invalid variable name '{value}'"
            )));
        }
        normalized.insert(value.to_string());
    }
    Ok(normalized.into_iter().collect())
}

fn is_valid_env_var_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn required_env_vars_for_provider(provider: &str) -> &'static [&'static str] {
    match provider {
        "codex" => &["OPENAI_API_KEY", "HOME", "PATH"],
        "claude" => &["ANTHROPIC_API_KEY", "HOME", "PATH"],
        _ => &[],
    }
}

#[derive(Debug, Deserialize)]
struct RawRuntimeConfig {
    execution: Option<RawExecutionConfig>,
}

#[derive(Debug, Deserialize)]
struct RawExecutionConfig {
    env: Option<RawExecutionEnvConfig>,
}

#[derive(Debug, Deserialize)]
struct RawExecutionEnvConfig {
    inherit: Option<bool>,
    pass: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::normalize_pass_list;

    #[test]
    fn normalize_pass_list_rejects_invalid_identifiers() {
        let err = normalize_pass_list(vec!["1INVALID".to_string()]).expect_err("must fail");
        assert!(err.to_string().contains("invalid variable name"));
    }

    #[test]
    fn normalize_pass_list_dedupes_and_sorts() {
        let values = normalize_pass_list(vec![
            "PATH".to_string(),
            "HOME".to_string(),
            "PATH".to_string(),
        ])
        .expect("normalize");
        assert_eq!(values, vec!["HOME".to_string(), "PATH".to_string()]);
    }
}

use serde_json::Value;

use crate::OrbitError;

const REDACTED_ENV_VALUE: &str = "[REDACTED_ENV]";

pub fn redact_sensitive_env_text(raw: &str) -> String {
    let mut redacted = raw.to_string();
    for secret in sensitive_env_values() {
        redacted = redacted.replace(&secret, REDACTED_ENV_VALUE);
    }
    redacted
}

pub fn redact_sensitive_env_option(raw: Option<String>) -> Option<String> {
    raw.map(|value| redact_sensitive_env_text(&value))
}

pub fn redact_sensitive_env_json(value: Value) -> Value {
    match value {
        Value::String(raw) => Value::String(redact_sensitive_env_text(&raw)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(redact_sensitive_env_json)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_sensitive_env_json(value)))
                .collect(),
        ),
        other => other,
    }
}

pub fn redact_sensitive_env_error(error: OrbitError) -> OrbitError {
    match error {
        OrbitError::PolicyDenied(message) => {
            OrbitError::PolicyDenied(redact_sensitive_env_text(&message))
        }
        OrbitError::ToolNotFound(message) => {
            OrbitError::ToolNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::TaskNotFound(message) => {
            OrbitError::TaskNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::TaskApprovalRequired(message) => {
            OrbitError::TaskApprovalRequired(redact_sensitive_env_text(&message))
        }
        OrbitError::SkillNotFound(message) => {
            OrbitError::SkillNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::JobNotFound(message) => {
            OrbitError::JobNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::JobRunNotFound(message) => {
            OrbitError::JobRunNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::ActivityNotFound(message) => {
            OrbitError::ActivityNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::AgentSessionNotFound(message) => {
            OrbitError::AgentSessionNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::IdentityNotFound(message) => {
            OrbitError::IdentityNotFound(redact_sensitive_env_text(&message))
        }
        OrbitError::InvalidInput(message) => {
            OrbitError::InvalidInput(redact_sensitive_env_text(&message))
        }
        OrbitError::SkillValidation(message) => {
            OrbitError::SkillValidation(redact_sensitive_env_text(&message))
        }
        OrbitError::IdentityValidation(message) => {
            OrbitError::IdentityValidation(redact_sensitive_env_text(&message))
        }
        OrbitError::JobValidation(message) => {
            OrbitError::JobValidation(redact_sensitive_env_text(&message))
        }
        OrbitError::AgentProtocolViolation(message) => {
            OrbitError::AgentProtocolViolation(redact_sensitive_env_text(&message))
        }
        OrbitError::UnsupportedAgentProvider(message) => {
            OrbitError::UnsupportedAgentProvider(redact_sensitive_env_text(&message))
        }
        OrbitError::Execution(message) => {
            OrbitError::Execution(redact_sensitive_env_text(&message))
        }
        OrbitError::Store(message) => OrbitError::Store(redact_sensitive_env_text(&message)),
        OrbitError::TaskStatusTransition(message) => {
            OrbitError::TaskStatusTransition(redact_sensitive_env_text(&message))
        }
        OrbitError::Io(message) => OrbitError::Io(redact_sensitive_env_text(&message)),
    }
}

fn sensitive_env_values() -> Vec<String> {
    let mut values = std::env::vars()
        .filter(|(name, value)| is_sensitive_env_name(name) && is_redactable_value(value))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();

    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

fn is_redactable_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 4
}

fn is_sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("PASSCODE")
        || upper.contains("API_KEY")
        || upper.ends_with("_KEY")
        || upper.contains("PRIVATE")
        || upper.contains("CREDENTIAL")
        || upper.contains("COOKIE")
        || upper.contains("SESSION")
        || upper.contains("BEARER")
        || upper.contains("AUTH")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::OrbitError;

    use super::{
        REDACTED_ENV_VALUE, redact_sensitive_env_error, redact_sensitive_env_json,
        redact_sensitive_env_text,
    };

    #[test]
    fn redacts_sensitive_env_values_in_text() {
        unsafe {
            std::env::set_var("TEST_API_KEY", "super-secret-token");
        }

        let redacted = redact_sensitive_env_text("token=super-secret-token");
        assert_eq!(redacted, format!("token={REDACTED_ENV_VALUE}"));
    }

    #[test]
    fn redacts_sensitive_env_values_in_json() {
        unsafe {
            std::env::set_var("TEST_GITHUB_TOKEN", "ghp_example_secret");
        }

        let value = json!({
            "stdout": "ghp_example_secret",
            "nested": ["prefix ghp_example_secret suffix"]
        });
        let redacted = redact_sensitive_env_json(value);
        assert_eq!(redacted["stdout"], REDACTED_ENV_VALUE);
        assert_eq!(
            redacted["nested"][0],
            format!("prefix {REDACTED_ENV_VALUE} suffix")
        );
    }

    #[test]
    fn redacts_sensitive_env_values_in_errors() {
        unsafe {
            std::env::set_var("TEST_SECRET_VALUE", "value-to-hide");
        }

        let error = OrbitError::Execution("command failed: value-to-hide".to_string());
        let redacted = redact_sensitive_env_error(error);
        assert_eq!(
            redacted.to_string(),
            format!("execution failed: command failed: {REDACTED_ENV_VALUE}")
        );
    }
}

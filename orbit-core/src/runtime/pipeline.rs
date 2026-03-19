use orbit_policy::PolicyContext;
use orbit_tools::ToolContext;
use orbit_types::{
    OrbitEvent, PolicyDecision, Role, redact_sensitive_env_error, redact_sensitive_env_json,
};
use serde_json::Value;

use crate::{OrbitError, OrbitRuntime};

impl OrbitRuntime {
    pub fn run_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
        self.run_tool_with_role(name, input, Role::Admin)
    }

    pub(crate) fn run_tool_with_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
    ) -> Result<Value, OrbitError> {
        self.run_tool_with_context_and_role(name, input, role, ToolContext::default())
    }

    pub(crate) fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        self.check_tool_enabled(name)?;

        if !tool_context.allowed_tools.is_empty()
            && !tool_context.allowed_tools.iter().any(|t| t == name)
        {
            self.with_mutation(|| {
                Ok((
                    (),
                    OrbitEvent::PolicyDenied {
                        tool: name.to_string(),
                    },
                ))
            })?;
            return Err(OrbitError::PolicyDenied(format!(
                "tool '{name}' is not in the activity allowlist"
            )));
        }

        let decision = self.context.policy.evaluate(&PolicyContext {
            entrypoint: "cli".to_string(),
            tool_name: Some(name.to_string()),
            role,
        });

        match decision {
            PolicyDecision::Deny { reason } => {
                self.with_mutation(|| {
                    Ok((
                        (),
                        OrbitEvent::PolicyDenied {
                            tool: name.to_string(),
                        },
                    ))
                })?;
                Err(OrbitError::PolicyDenied(reason))
            }
            PolicyDecision::Allow => {
                let output = self
                    .context
                    .registry
                    .execute(name, &tool_context, input)
                    .map_err(redact_sensitive_env_error)?;
                let output = redact_sensitive_env_json(output);

                self.with_mutation(|| {
                    Ok((
                        (),
                        OrbitEvent::ToolExecuted {
                            name: name.to_string(),
                        },
                    ))
                })?;

                Ok(output)
            }
        }
    }

    pub fn run_tool_dry_run(&self, name: &str, input: &Value) -> Result<DryRunResult, OrbitError> {
        self.check_tool_enabled(name)?;

        let schema = self
            .context
            .registry
            .get_schema(name)
            .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;

        let decision = self.context.policy.evaluate(&PolicyContext {
            entrypoint: "cli".to_string(),
            tool_name: Some(name.to_string()),
            role: Role::Admin,
        });

        let policy_allowed = matches!(decision, PolicyDecision::Allow);

        // Validate required parameters are present
        let mut missing_params = Vec::new();
        if let Some(obj) = input.as_object() {
            for param in &schema.parameters {
                if param.required && !obj.contains_key(&param.name) {
                    missing_params.push(param.name.clone());
                }
            }
        } else if !schema.parameters.is_empty() {
            for param in &schema.parameters {
                if param.required {
                    missing_params.push(param.name.clone());
                }
            }
        }

        Ok(DryRunResult {
            tool_name: name.to_string(),
            policy_allowed,
            missing_params,
        })
    }

    fn check_tool_enabled(&self, name: &str) -> Result<(), OrbitError> {
        if let Some(stored) = self.context.tool_store.get_tool(name)?
            && !stored.enabled
        {
            return Err(OrbitError::Execution(format!(
                "tool '{name}' is disabled; enable it with: orbit tool enable {name}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DryRunResult {
    pub tool_name: String,
    pub policy_allowed: bool,
    pub missing_params: Vec<String>,
}

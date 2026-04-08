use std::collections::HashMap;

use orbit_types::OrbitError;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub input: Value,
    pub env: HashMap<String, String>,
    pub workspace_path: Option<String>,
}

pub fn render(template: &str, ctx: &TemplateContext) -> Result<String, OrbitError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let end = after_start.find("}}").ok_or_else(|| {
            OrbitError::InvalidInput(format!("unterminated template token in '{template}'"))
        })?;
        let token = after_start[..end].trim();
        output.push_str(&resolve_token(token, ctx)?);
        remaining = &after_start[end + 2..];
    }

    output.push_str(remaining);
    Ok(output)
}

fn resolve_token(token: &str, ctx: &TemplateContext) -> Result<String, OrbitError> {
    if token == "workspace_path" {
        return ctx.workspace_path.clone().ok_or_else(|| {
            OrbitError::InvalidInput("workspace_path is unavailable in this context".to_string())
        });
    }

    let mut parts = token.split('.');
    let namespace = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OrbitError::InvalidInput("empty template token".to_string()))?;
    let path = parts.collect::<Vec<_>>();
    if path.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "template token '{token}' must include a key path"
        )));
    }

    match namespace {
        "input" => resolve_input_path(Some(&ctx.input), &path),
        "env" => {
            if path.len() != 1 {
                return Err(OrbitError::InvalidInput(format!(
                    "env template token '{token}' must reference a single variable"
                )));
            }
            ctx.env.get(path[0]).cloned().ok_or_else(|| {
                OrbitError::InvalidInput(format!("missing environment variable '{}'", path[0]))
            })
        }
        "secrets" => Err(OrbitError::InvalidInput(
            "secrets namespace is not yet supported".to_string(),
        )),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown template namespace: {other}"
        ))),
    }
}

fn resolve_input_path(input: Option<&Value>, path: &[&str]) -> Result<String, OrbitError> {
    let mut current = input.ok_or_else(|| {
        OrbitError::InvalidInput("input template namespace requires an object input".to_string())
    })?;
    for segment in path {
        current = current.get(segment).ok_or_else(|| {
            OrbitError::InvalidInput(format!("missing input value for '{}'", path.join(".")))
        })?;
    }

    match current {
        Value::String(value) => Ok(value.clone()),
        Value::Null => Ok("null".to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(current)
            .map_err(|error| OrbitError::InvalidInput(error.to_string())),
    }
}

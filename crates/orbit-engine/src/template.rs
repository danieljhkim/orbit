use std::collections::HashMap;

use orbit_common::types::OrbitError;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub input: Value,
    pub env: HashMap<String, String>,
    pub workspace_path: Option<String>,
    pub item: Option<Value>,
    pub iteration: Option<u32>,
    /// Accumulated outputs from completed steps, keyed by step id (or target_id).
    pub steps: HashMap<String, Value>,
}

pub fn render(template: &str, ctx: &TemplateContext) -> Result<String, OrbitError> {
    render_with_tokens(template, ctx, |output, value| {
        output.push_str(value);
    })
}

/// Render a shell command while passing template values as positional argv.
///
/// The returned command string contains shell parameter references (`${1}`,
/// `${2}`, ...) and the returned vector contains the corresponding values.
pub fn render_shell_command(
    template: &str,
    ctx: &TemplateContext,
) -> Result<(String, Vec<String>), OrbitError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    let mut values = Vec::new();
    let mut quote_state = ShellQuoteState::Unquoted;

    while let Some(start) = remaining.find("{{") {
        let literal = &remaining[..start];
        output.push_str(literal);
        quote_state = advance_shell_quote_state(quote_state, literal);

        let after_start = &remaining[start + 2..];
        let end = after_start.find("}}").ok_or_else(|| {
            OrbitError::InvalidInput(format!("unterminated template token in '{template}'"))
        })?;
        let token = after_start[..end].trim();
        let value = resolve_token(token, ctx)?;
        values.push(value.to_string());
        push_shell_positional(&mut output, values.len(), quote_state);
        remaining = &after_start[end + 2..];
    }

    output.push_str(remaining);
    Ok((output, values))
}

fn render_with_tokens<F>(
    template: &str,
    ctx: &TemplateContext,
    mut write_token: F,
) -> Result<String, OrbitError>
where
    F: FnMut(&mut String, &str),
{
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let end = after_start.find("}}").ok_or_else(|| {
            OrbitError::InvalidInput(format!("unterminated template token in '{template}'"))
        })?;
        let token = after_start[..end].trim();
        let value = resolve_token(token, ctx)?;
        write_token(&mut output, &value);
        remaining = &after_start[end + 2..];
    }

    output.push_str(remaining);
    Ok(output)
}

#[derive(Clone, Copy)]
enum ShellQuoteState {
    Unquoted,
    Single,
    Double,
}

fn push_shell_positional(output: &mut String, index: usize, quote_state: ShellQuoteState) {
    match quote_state {
        ShellQuoteState::Unquoted => {
            output.push_str("\"${");
            output.push_str(&index.to_string());
            output.push_str("}\"");
        }
        ShellQuoteState::Single => {
            output.push_str("'\"${");
            output.push_str(&index.to_string());
            output.push_str("}\"'");
        }
        ShellQuoteState::Double => {
            output.push_str("${");
            output.push_str(&index.to_string());
            output.push('}');
        }
    }
}

fn advance_shell_quote_state(mut state: ShellQuoteState, literal: &str) -> ShellQuoteState {
    let mut chars = literal.chars();
    while let Some(ch) = chars.next() {
        match state {
            ShellQuoteState::Unquoted => match ch {
                '\\' => {
                    let _ = chars.next();
                }
                '\'' => state = ShellQuoteState::Single,
                '"' => state = ShellQuoteState::Double,
                _ => {}
            },
            ShellQuoteState::Single => {
                if ch == '\'' {
                    state = ShellQuoteState::Unquoted;
                }
            }
            ShellQuoteState::Double => match ch {
                '\\' => {
                    let _ = chars.next();
                }
                '"' => state = ShellQuoteState::Unquoted,
                _ => {}
            },
        }
    }
    state
}

fn resolve_token(token: &str, ctx: &TemplateContext) -> Result<String, OrbitError> {
    if token == "workspace_path" {
        return ctx.workspace_path.clone().ok_or_else(|| {
            OrbitError::InvalidInput("workspace_path is unavailable in this context".to_string())
        });
    }
    if token == "item" {
        return resolve_input_path(ctx.item.as_ref(), &[]);
    }
    if token == "iteration" {
        return ctx.iteration.map(|value| value.to_string()).ok_or_else(|| {
            OrbitError::InvalidInput("iteration is unavailable in this context".to_string())
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
        "item" => resolve_input_path(ctx.item.as_ref(), &path),
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
        "steps" => {
            // steps.<step_id>.<namespace>.<field>...
            // where <namespace> is "state" or "output".
            if path.len() < 2 {
                return Err(OrbitError::InvalidInput(format!(
                    "steps template token '{token}' must be steps.<id>.state.<field> or steps.<id>.output.<field>"
                )));
            }
            let step_id = path[0];
            let step_value = ctx.steps.get(step_id).ok_or_else(|| {
                OrbitError::InvalidInput(format!("no data recorded for step '{step_id}'"))
            })?;
            let sub_namespace = path[1];
            match sub_namespace {
                "state" | "output" => {
                    let sub_value = step_value.get(sub_namespace).ok_or_else(|| {
                        OrbitError::InvalidInput(format!(
                            "step '{step_id}' has no '{sub_namespace}' data"
                        ))
                    })?;
                    if path.len() == 2 {
                        // steps.<id>.state or steps.<id>.output — return the whole sub-object
                        resolve_input_path(Some(sub_value), &[])
                    } else {
                        resolve_input_path(Some(sub_value), &path[2..])
                    }
                }
                other => Err(OrbitError::InvalidInput(format!(
                    "unknown steps sub-namespace '{other}' in '{token}'; expected 'state' or 'output'"
                ))),
            }
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

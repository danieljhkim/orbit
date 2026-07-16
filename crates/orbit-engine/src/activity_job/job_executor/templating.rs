use super::*;

pub(super) fn render_input(
    default_input: Option<&Value>,
    base_input: &Value,
    tctx: &TemplateContext,
) -> Result<Value, DispatchError> {
    let src = default_input.cloned().unwrap_or_else(|| base_input.clone());
    render_value(&src, tctx)
}

pub(super) fn merge_job_input(default_input: Option<&Value>, input: &Value) -> Value {
    match (default_input, input) {
        (Some(defaults), Value::Null) => defaults.clone(),
        (Some(Value::Object(defaults)), Value::Object(explicit)) => {
            let mut merged = defaults.clone();
            for (key, value) in explicit {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        _ => input.clone(),
    }
}

pub(super) fn render_items_expression(
    expression: &str,
    tctx: &TemplateContext,
    label: &str,
) -> Result<Vec<Value>, DispatchError> {
    let rendered = template::render(expression, tctx)
        .map_err(|err| DispatchError::JobExecution(format!("{label} render: {err}")))?;
    Ok(serde_json::from_str(&rendered).unwrap_or_else(|_| {
        rendered
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|segment| !segment.is_empty())
            .map(|segment| Value::String(segment.to_string()))
            .collect()
    }))
}

/// Recursive template render: resolves `{{ ... }}` tokens in any string
/// within a JSON tree. Non-strings pass through unchanged.
pub(super) fn render_value(v: &Value, tctx: &TemplateContext) -> Result<Value, DispatchError> {
    match v {
        Value::String(s) if s.contains("{{") => {
            // L-0091: Exact step-output templates forward typed JSON values.
            // ORB-10241: Preserve the source JSON type for an exact step-output
            // reference. Rendering through text and parsing it again turns a
            // string such as PR number "618" into the number 618, which then
            // fails downstream string validation despite being present.
            if let Some(value) = exact_step_template_value(s, tctx) {
                return Ok(value);
            }
            let rendered = template::render(s, tctx)
                .map_err(|err| DispatchError::JobExecution(format!("template render: {err}")))?;
            // Try to parse back to a JSON value (numbers, bools, arrays);
            // fall back to string if parse fails.
            Ok(serde_json::from_str::<Value>(&rendered).unwrap_or(Value::String(rendered)))
        }
        Value::Array(arr) => {
            let out: Result<Vec<_>, _> = arr.iter().map(|x| render_value(x, tctx)).collect();
            Ok(Value::Array(out?))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), render_value(v, tctx)?);
            }
            Ok(Value::Object(out))
        }
        _ => Ok(v.clone()),
    }
}

fn exact_step_template_value(template: &str, tctx: &TemplateContext) -> Option<Value> {
    let token = template.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    if token.contains("{{") || token.contains("}}") {
        return None;
    }

    let mut path = token.split('.');
    if path.next()? != "steps" {
        return None;
    }
    let step_id = path.next()?;
    let namespace = path.next()?;
    if namespace != "output" && namespace != "state" {
        return None;
    }

    let mut value = tctx.steps.get(step_id)?.get(namespace)?;
    for segment in path {
        value = value.get(segment)?;
    }
    Some(value.clone())
}

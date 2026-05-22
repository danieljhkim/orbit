use serde_json::Value;

use crate::types::OrbitError;

pub const RETIRED_TASK_ADD_INPUT_FIELDS: &[&str] = &[
    "plan",
    "status",
    "crew",
    "parent_id",
    "source_task_id",
    "external_refs",
    "context",
    "comment",
    "dependencies",
];

pub fn strip_retired_task_add_input_fields(input: &mut Value) -> Vec<&'static str> {
    let Some(object) = input.as_object_mut() else {
        return Vec::new();
    };

    let mut ignored = Vec::new();
    for field in RETIRED_TASK_ADD_INPUT_FIELDS {
        if object.remove(*field).is_some() {
            ignored.push(*field);
        }
    }
    ignored
}

pub fn required_string(
    input: &Value,
    keys: &[&str],
    canonical: &str,
) -> Result<String, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return Ok(trimmed.to_string());
        }
    }
    Err(OrbitError::InvalidInput(format!("missing `{canonical}`")))
}

pub fn optional_string(input: &Value, key: &str) -> Result<Option<String>, OrbitError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

pub fn optional_raw_string(input: &Value, key: &str) -> Result<Option<String>, OrbitError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            Ok(Some(raw.to_string()))
        }
    }
}

pub fn optional_string_alias(input: &Value, keys: &[&str]) -> Result<Option<String>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return Ok(Some(trimmed.to_string()));
        }
    }
    Ok(None)
}

pub fn optional_u32_alias(input: &Value, keys: &[&str]) -> Result<Option<u32>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = match value {
                Value::String(value) => value.trim().to_string(),
                Value::Number(value) => value.to_string(),
                _ => {
                    return Err(OrbitError::InvalidInput(format!(
                        "`{key}` must be a string or integer"
                    )));
                }
            };
            if raw.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return raw.parse::<u32>().map(Some).map_err(|error| {
                OrbitError::InvalidInput(format!("`{key}` must be an unsigned integer: {error}"))
            });
        }
    }
    Ok(None)
}

pub fn optional_string_list_alias(
    input: &Value,
    keys: &[&str],
) -> Result<Option<Vec<String>>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            return match value {
                Value::String(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        Err(OrbitError::InvalidInput(format!(
                            "`{key}` must not be empty"
                        )))
                    } else if let Some(recovered) = decode_json_string_array(trimmed) {
                        Ok(Some(recovered))
                    } else {
                        Ok(Some(vec![trimmed.to_string()]))
                    }
                }
                Value::Array(items) => {
                    if let [Value::String(raw)] = items.as_slice()
                        && let Some(recovered) = decode_json_string_array(raw.trim())
                    {
                        return Ok(Some(recovered));
                    }
                    let mut values = Vec::with_capacity(items.len());
                    for item in items {
                        let raw = item.as_str().ok_or_else(|| {
                            OrbitError::InvalidInput(format!("`{key}` entries must be strings"))
                        })?;
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            return Err(OrbitError::InvalidInput(format!(
                                "`{key}` entries must not be empty"
                            )));
                        }
                        values.push(trimmed.to_string());
                    }
                    Ok(Some(values))
                }
                _ => Err(OrbitError::InvalidInput(format!(
                    "`{key}` must be a string or array of strings"
                ))),
            };
        }
    }
    Ok(None)
}

pub fn optional_csv_or_string_list_alias(
    input: &Value,
    keys: &[&str],
) -> Result<Option<Vec<String>>, OrbitError> {
    optional_string_list_alias(input, keys).map(|values| {
        values.map(|items| {
            items
                .into_iter()
                .flat_map(|item| split_csv(&item))
                .collect::<Vec<_>>()
        })
    })
}

pub fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Recover a string array that an MCP client serialized as a JSON-encoded
/// scalar string. Some clients flatten arrays into JSON strings when a tool
/// schema is `anyOf:[array,string]`; without this recovery, the parser would
/// store the entire JSON blob as a single list element. Returns `Some(values)`
/// only when `raw` decodes to a JSON array of non-empty strings; otherwise
/// returns `None` so callers fall back to treating `raw` as plain text.
fn decode_json_string_array(raw: &str) -> Option<Vec<String>> {
    if !(raw.starts_with('[') && raw.ends_with(']')) {
        return None;
    }
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let Value::Array(items) = parsed else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(text) = item else {
            return None;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        values.push(trimmed.to_string());
    }
    Some(values)
}

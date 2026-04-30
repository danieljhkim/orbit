use serde_json::Value;

use crate::types::OrbitError;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn optional_string_list_accepts_scalar_string() {
        assert_eq!(
            optional_string_list_alias(&json!({"values":"one"}), &["values"]).unwrap(),
            Some(vec!["one".to_string()])
        );
    }

    #[test]
    fn optional_string_list_preserves_array_behavior() {
        assert_eq!(
            optional_string_list_alias(&json!({"values":["one", "two"]}), &["values"]).unwrap(),
            Some(vec!["one".to_string(), "two".to_string()])
        );
    }

    #[test]
    fn optional_string_list_rejects_non_string_shapes() {
        let error = optional_string_list_alias(&json!({"values":{"one":true}}), &["values"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("`values` must be a string or array of strings"));
    }

    #[test]
    fn optional_string_list_recovers_json_encoded_array() {
        assert_eq!(
            optional_string_list_alias(&json!({"values": "[\"a\",\"b\"]"}), &["values"]).unwrap(),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn optional_string_list_recovers_json_encoded_array_with_whitespace() {
        assert_eq!(
            optional_string_list_alias(&json!({"values": "  [\"a\", \"b\"]  "}), &["values"])
                .unwrap(),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn optional_string_list_keeps_plain_string_with_brackets() {
        assert_eq!(
            optional_string_list_alias(&json!({"values": "[draft] note"}), &["values"]).unwrap(),
            Some(vec!["[draft] note".to_string()])
        );
    }

    #[test]
    fn optional_string_list_falls_back_for_heterogeneous_json_arrays() {
        assert_eq!(
            optional_string_list_alias(&json!({"values": "[\"a\", 5]"}), &["values"]).unwrap(),
            Some(vec!["[\"a\", 5]".to_string()])
        );
    }

    #[test]
    fn optional_string_list_falls_back_for_recovered_empty_strings() {
        assert_eq!(
            optional_string_list_alias(&json!({"values": "[\"a\", \"\"]"}), &["values"]).unwrap(),
            Some(vec!["[\"a\", \"\"]".to_string()])
        );
    }

    #[test]
    fn optional_csv_or_string_list_recovers_json_encoded_selectors() {
        let recovered = optional_csv_or_string_list_alias(
            &json!({"context_files": "[\"file:src/lib.rs\", \"file:src/main.rs\"]"}),
            &["context_files"],
        )
        .unwrap();
        assert_eq!(
            recovered,
            Some(vec![
                "file:src/lib.rs".to_string(),
                "file:src/main.rs".to_string()
            ])
        );
    }
}

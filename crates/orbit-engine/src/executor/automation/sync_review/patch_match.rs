use std::collections::HashMap;

use orbit_types::OrbitError;
use serde_json::Value;

pub(super) type PrFilePatchMap = HashMap<String, Option<String>>;

pub(super) fn parse_pr_file_patches(stdout: &str) -> Result<PrFilePatchMap, OrbitError> {
    let payload: Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to parse gh api pull request files output: {error}"
        ))
    })?;

    let mut patches = HashMap::new();
    for item in flatten_paginated_items(payload, "pull request files")? {
        let Value::Object(file) = item else {
            return Err(OrbitError::Execution(
                "gh api pull request files returned non-object item".to_string(),
            ));
        };
        let filename = file
            .get("filename")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OrbitError::Execution(
                    "gh api pull request files returned item without filename".to_string(),
                )
            })?;
        let patch = file.get("patch").and_then(Value::as_str).map(String::from);
        patches.insert(filename.to_string(), patch);
    }

    Ok(patches)
}

pub(super) fn patch_supports_right_side_line(patch: &str, target_line: u64) -> bool {
    if target_line == 0 {
        return false;
    }

    let mut current_new_line: Option<u64> = None;

    for line in patch.lines() {
        if let Some(start_line) = parse_hunk_new_start(line) {
            current_new_line = Some(start_line);
            continue;
        }

        let Some(new_line) = current_new_line.as_mut() else {
            continue;
        };

        match line.as_bytes().first().copied() {
            Some(b' ') | Some(b'+') => {
                if *new_line == target_line {
                    return true;
                }
                *new_line += 1;
            }
            Some(b'-') => {}
            _ => {}
        }
    }

    false
}

fn flatten_paginated_items(payload: Value, label: &str) -> Result<Vec<Value>, OrbitError> {
    match payload {
        Value::Array(items) => {
            let mut flattened = Vec::new();
            for item in items {
                match item {
                    Value::Array(page) => flattened.extend(page),
                    Value::Object(_) => flattened.push(item),
                    other => {
                        return Err(OrbitError::Execution(format!(
                            "gh api {label} returned unexpected item type: {}",
                            json_type_name(&other)
                        )));
                    }
                }
            }
            Ok(flattened)
        }
        other => Err(OrbitError::Execution(format!(
            "gh api {label} returned unexpected payload type: {}",
            json_type_name(&other)
        ))),
    }
}

fn parse_hunk_new_start(line: &str) -> Option<u64> {
    if !line.starts_with("@@") {
        return None;
    }

    line.split_whitespace()
        .find(|segment| segment.starts_with('+'))
        .and_then(|segment| segment.trim_start_matches('+').split(',').next())
        .and_then(|start| start.parse::<u64>().ok())
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

use serde_json::{Map, Value, json};

pub fn schema_with_identity(extra_properties: Map<String, Value>, required: &[&str]) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "identity_id".to_string(),
        json!({ "type": "string", "minLength": 1 }),
    );
    for (key, value) in extra_properties {
        properties.insert(key, value);
    }

    let mut required_fields: Vec<String> = vec!["identity_id".to_string()];
    for field in required {
        if *field != "identity_id" {
            required_fields.push((*field).to_string());
        }
    }

    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required_fields,
    })
}

pub fn obj(props: &[(&str, Value)]) -> Map<String, Value> {
    let mut map = Map::new();
    for (key, value) in props {
        map.insert((*key).to_string(), value.clone());
    }
    map
}

pub fn str_schema() -> Value {
    json!({ "type": "string" })
}

pub fn bool_schema() -> Value {
    json!({ "type": "boolean" })
}

pub fn int_schema() -> Value {
    json!({ "type": "integer" })
}

pub fn any_schema() -> Value {
    json!({})
}

pub fn array_of_string_schema() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

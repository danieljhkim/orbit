use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use orbit_common::types::{
    FrictionRecord, FrictionStatus, OrbitError, optional_csv_or_string_list_alias, optional_string,
    required_string,
};
use orbit_store::friction_store::{
    FrictionAddParams, FrictionListFilter, FrictionUpdateParams, StoredFrictionRecord,
    add_friction, friction_stats, friction_tags, list_frictions, resolve_friction, show_friction,
    update_friction,
};
use serde_json::{Value, json};

use crate::OrbitRuntime;

pub(super) fn add(
    runtime: &OrbitRuntime,
    input: Value,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let body = required_string(&input, &["body", "description"], "body")?;
    let tags = optional_csv_or_string_list_alias(&input, &["tags", "tag"])?.unwrap_or_default();
    let during_task = optional_string(&input, "during_task")?
        .or_else(|| optional_string(&input, "task_id").ok().flatten());
    let model = model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("orbit.friction.add requires `model`".to_string())
        })?;
    let stored = add_friction(
        &runtime.data_root().join("frictions"),
        FrictionAddParams {
            model,
            body,
            tags,
            during_task,
            created_at: Utc::now(),
        },
    )?;
    record_to_json(stored)
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let month_bounds = optional_string(&input, "month")?
        .map(|raw| parse_month_bounds(&raw))
        .transpose()?;
    let (month_from, month_to) = month_bounds
        .map(|(from, to)| (Some(from), Some(to)))
        .unwrap_or((None, None));
    let filter = FrictionListFilter {
        model: optional_string(&input, "model")?,
        status: optional_string(&input, "status")?
            .map(|status| parse_status(&status))
            .transpose()?,
        tag: optional_string(&input, "tag")?,
        q: optional_string(&input, "q")?,
        from: optional_string(&input, "from")?
            .map(|raw| parse_timestamp("from", &raw))
            .transpose()?
            .or(month_from),
        to: optional_string(&input, "to")?
            .map(|raw| parse_timestamp("to", &raw))
            .transpose()?
            .or(month_to),
    };
    let limit = optional_usize(&input, "limit")?;
    let offset = optional_usize(&input, "offset")?.unwrap_or(0);
    let records = list_frictions(&runtime.data_root().join("frictions"), &filter)?;
    Ok(Value::Array(
        records
            .into_iter()
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .map(record_to_json)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let Some(stored) = show_friction(&runtime.data_root().join("frictions"), &id)? else {
        return Err(OrbitError::InvalidInput(format!(
            "friction record not found: {id}"
        )));
    };
    record_to_json(stored)
}

pub(super) fn stats(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let tasks = runtime.list_tasks()?;
    friction_stats(&runtime.data_root().join("frictions"), &tasks)
}

pub(super) fn tags(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    Ok(json!(friction_tags(
        &runtime.data_root().join("frictions")
    )?))
}

pub(super) fn update(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let status = optional_string(&input, "status")?
        .map(|status| parse_status(&status))
        .transpose()?;
    let tags = optional_csv_or_string_list_alias(&input, &["tags", "tag"])?;
    if status.is_none() && tags.is_none() {
        return Err(OrbitError::InvalidInput(
            "orbit.friction.update requires `status` or `tags`".to_string(),
        ));
    }
    let stored = update_friction(
        &runtime.data_root().join("frictions"),
        &id,
        FrictionUpdateParams {
            status,
            tags,
            updated_at: Utc::now(),
        },
    )?;
    record_to_json(stored)
}

pub(super) fn resolve(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let stored = resolve_friction(&runtime.data_root().join("frictions"), &id, Utc::now())?;
    record_to_json(stored)
}

fn parse_timestamp(field: &str, raw: &str) -> Result<DateTime<Utc>, OrbitError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| OrbitError::InvalidInput(format!("`{field}` must be RFC3339: {error}")))
}

fn parse_month_bounds(raw: &str) -> Result<(DateTime<Utc>, DateTime<Utc>), OrbitError> {
    let bytes = raw.as_bytes();
    let format_ok = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    if !format_ok {
        return Err(OrbitError::InvalidInput(format!(
            "`month` must be in YYYY-MM format, got '{raw}'"
        )));
    }
    let year = raw[..4].parse::<i32>().map_err(|_| {
        OrbitError::InvalidInput(format!("invalid year component in `month`: {raw}"))
    })?;
    let month = raw[5..].parse::<u32>().map_err(|_| {
        OrbitError::InvalidInput(format!("invalid month component in `month`: {raw}"))
    })?;
    if !(1..=12).contains(&month) {
        return Err(OrbitError::InvalidInput(format!(
            "`month` component must be 01-12, got '{raw}'"
        )));
    }
    let start = Utc
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid `month`: {raw}")))?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end_exclusive = Utc
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid `month`: {raw}")))?;
    Ok((start, end_exclusive - chrono::Duration::nanoseconds(1)))
}

fn parse_status(raw: &str) -> Result<FrictionStatus, OrbitError> {
    FrictionStatus::from_str(raw)
        .map_err(|error| OrbitError::InvalidInput(format!("`status` {error}")))
}

fn optional_usize(input: &Value, field: &str) -> Result<Option<usize>, OrbitError> {
    let Some(value) = input.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let n = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| OrbitError::InvalidInput(format!("`{field}` must be a non-negative integer")))?;
    Ok(Some(n as usize))
}

fn record_to_json(stored: StoredFrictionRecord) -> Result<Value, OrbitError> {
    let mut value = serde_json::to_value(&stored.record)
        .map_err(|error| OrbitError::Store(format!("serialize friction record: {error}")))?;
    if let Some(object) = value.as_object_mut() {
        object.insert("path".to_string(), json!(stored.path.to_string_lossy()));
        object.insert("title".to_string(), json!(record_title(&stored.record)));
    }
    Ok(value)
}

fn record_title(record: &FrictionRecord) -> String {
    record
        .body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| record.id.clone())
}

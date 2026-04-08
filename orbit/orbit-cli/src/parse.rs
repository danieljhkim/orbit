use chrono::{DateTime, Utc};

pub fn csv_to_vec(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Parses a duration-relative string like "1h", "90d", "30m", "2w"
/// or an RFC3339/naive timestamp into a `DateTime<Utc>`.
/// For bare durations, the result is `now - duration`.
pub fn parse_since(raw: &str) -> Result<DateTime<Utc>, orbit_core::OrbitError> {
    let value = raw.trim();

    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Ok(naive.and_utc());
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Ok(naive.and_utc());
    }

    let (num_raw, unit_raw) = split_duration_components(value)?;
    let num: i64 = num_raw.parse().map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!("invalid duration number: {num_raw}"))
    })?;

    if num <= 0 {
        return Err(orbit_core::OrbitError::InvalidInput(
            "duration must be positive".to_string(),
        ));
    }

    let seconds = match unit_raw {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        "w" => num * 604800,
        other => {
            return Err(orbit_core::OrbitError::InvalidInput(format!(
                "unknown duration suffix: {other} (use s/m/h/d/w)"
            )));
        }
    };

    Ok(Utc::now() - chrono::Duration::seconds(seconds))
}

pub fn parse_duration_seconds(raw: &str) -> Result<u64, orbit_core::OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(orbit_core::OrbitError::InvalidInput(
            "duration must not be empty".to_string(),
        ));
    }

    let split_at = value
        .find(|c: char| c.is_alphabetic())
        .ok_or_else(|| orbit_core::OrbitError::InvalidInput(format!("invalid duration: {raw}")))?;
    let (num_raw, unit_raw) = value.split_at(split_at);

    let num: u64 = num_raw.parse().map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!("invalid duration number: {raw}"))
    })?;

    let seconds = match unit_raw {
        "s" => num,
        "m" => num.saturating_mul(60),
        "h" => num.saturating_mul(3600),
        "d" => num.saturating_mul(86400),
        "w" => num.saturating_mul(604800),
        _ => {
            return Err(orbit_core::OrbitError::InvalidInput(format!(
                "invalid duration unit: {unit_raw} (expected s/m/h/d/w)"
            )));
        }
    };

    Ok(seconds)
}

fn split_duration_components(input: &str) -> Result<(&str, &str), orbit_core::OrbitError> {
    let split_at = input.find(|c: char| c.is_alphabetic()).ok_or_else(|| {
        orbit_core::OrbitError::InvalidInput(format!("invalid duration format: {input}"))
    })?;

    let (num, suffix) = input.split_at(split_at);
    if num.is_empty() {
        return Err(orbit_core::OrbitError::InvalidInput(format!(
            "missing number in duration: {input}"
        )));
    }

    Ok((num, suffix))
}

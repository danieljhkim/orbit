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

    let seconds = parse_duration_seconds(value)?;
    let seconds = i64::try_from(seconds).map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!(
            "duration '{raw}' is too large to convert into a timestamp"
        ))
    })?;
    let duration = chrono::Duration::try_seconds(seconds).ok_or_else(|| {
        orbit_core::OrbitError::InvalidInput(format!(
            "duration '{raw}' is too large to convert into a timestamp"
        ))
    })?;
    Utc::now().checked_sub_signed(duration).ok_or_else(|| {
        orbit_core::OrbitError::InvalidInput(format!(
            "duration '{raw}' is too large to convert into a timestamp"
        ))
    })
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
        "s" => Some(num),
        "m" => num.checked_mul(60),
        "h" => num.checked_mul(3600),
        "d" => num.checked_mul(86400),
        "w" => num.checked_mul(604800),
        _ => {
            return Err(orbit_core::OrbitError::InvalidInput(format!(
                "invalid duration unit: {unit_raw} (expected s/m/h/d/w)"
            )));
        }
    }
    .ok_or_else(|| {
        orbit_core::OrbitError::InvalidInput(format!("duration '{raw}' is too large to represent"))
    })?;

    Ok(seconds)
}

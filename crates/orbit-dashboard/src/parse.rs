use chrono::{DateTime, Utc};

/// Parses a duration-relative string like "1h", "90d", "30m", "2w"
/// or an RFC3339/naive timestamp into a `DateTime<Utc>`.
/// For bare durations, the result is `now - duration`.
pub(crate) fn parse_since(raw: &str) -> Result<DateTime<Utc>, orbit_core::OrbitError> {
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

pub(crate) fn parse_duration_seconds(raw: &str) -> Result<u64, orbit_core::OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(orbit_core::OrbitError::InvalidInput(
            "duration string is empty".to_string(),
        ));
    }

    let (num_str, unit) = if let Some(stripped) = value.strip_suffix('s') {
        (stripped, "s")
    } else if let Some(stripped) = value.strip_suffix('m') {
        (stripped, "m")
    } else if let Some(stripped) = value.strip_suffix('h') {
        (stripped, "h")
    } else if let Some(stripped) = value.strip_suffix('d') {
        (stripped, "d")
    } else if let Some(stripped) = value.strip_suffix('w') {
        (stripped, "w")
    } else {
        // bare number = seconds
        return value.parse::<u64>().map_err(|_| {
            orbit_core::OrbitError::InvalidInput(format!("invalid duration '{raw}'"))
        });
    };

    let num: u64 = num_str.parse().map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!("invalid number in duration '{raw}'"))
    })?;

    let secs = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        "w" => num * 86400 * 7,
        _ => unreachable!(),
    };
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339() {
        // test-only: unwrap is acceptable for deterministic RFC3339 input in an isolated unit test
        #[allow(clippy::unwrap_used)]
        let ts = parse_since("2025-01-01T00:00:00Z").unwrap();
        assert_eq!(ts.timestamp(), 1735689600);
    }

    #[test]
    fn parses_duration() {
        // This is a bit racy on "now", but we can assert it's recent.
        // test-only: unwrap is acceptable for deterministic duration input in an isolated unit test
        #[allow(clippy::unwrap_used)]
        let ts = parse_since("10s").unwrap();
        let now = Utc::now();
        assert!(now.signed_duration_since(ts).num_seconds() >= 9);
    }
}

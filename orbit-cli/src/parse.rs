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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_seconds_units() {
        assert_eq!(parse_duration_seconds("30s").unwrap(), 30);
        assert_eq!(parse_duration_seconds("5m").unwrap(), 300);
        assert_eq!(parse_duration_seconds("2h").unwrap(), 7200);
        assert_eq!(parse_duration_seconds("1d").unwrap(), 86400);
        assert_eq!(parse_duration_seconds("1w").unwrap(), 604800);
    }

    #[test]
    fn parse_duration_seconds_whitespace() {
        assert_eq!(parse_duration_seconds("  15m  ").unwrap(), 900);
    }

    #[test]
    fn parse_duration_seconds_empty() {
        assert!(parse_duration_seconds("").is_err());
        assert!(parse_duration_seconds("   ").is_err());
    }

    #[test]
    fn parse_duration_seconds_invalid_unit() {
        assert!(parse_duration_seconds("5x").is_err());
    }

    #[test]
    fn parse_duration_seconds_non_numeric() {
        assert!(parse_duration_seconds("abcm").is_err());
    }

    #[test]
    fn parse_duration_seconds_no_unit() {
        assert!(parse_duration_seconds("300").is_err());
    }

    #[test]
    fn parse_since_hours() {
        let result = parse_since("1h").expect("parse 1h");
        let diff = Utc::now() - result;
        assert!((diff.num_seconds() - 3600).abs() < 5);
    }

    #[test]
    fn parse_since_days() {
        let result = parse_since("90d").expect("parse 90d");
        let diff = Utc::now() - result;
        assert!((diff.num_seconds() - 90 * 86400).abs() < 5);
    }

    #[test]
    fn parse_since_minutes() {
        let result = parse_since("30m").expect("parse 30m");
        let diff = Utc::now() - result;
        assert!((diff.num_seconds() - 1800).abs() < 5);
    }

    #[test]
    fn parse_since_weeks() {
        let result = parse_since("2w").expect("parse 2w");
        let diff = Utc::now() - result;
        assert!((diff.num_seconds() - 2 * 604800).abs() < 5);
    }

    #[test]
    fn parse_since_seconds() {
        let result = parse_since("60s").expect("parse 60s");
        let diff = Utc::now() - result;
        assert!((diff.num_seconds() - 60).abs() < 5);
    }

    #[test]
    fn parse_since_rfc3339() {
        let result = parse_since("2026-01-15T10:00:00+00:00").expect("parse rfc3339");
        assert!(result.to_rfc3339().starts_with("2026-01-15"));
    }

    #[test]
    fn parse_since_naive_timestamp() {
        let result = parse_since("2026-01-15T10:00:00").expect("parse naive");
        assert!(result.to_rfc3339().starts_with("2026-01-15"));
    }

    #[test]
    fn parse_since_invalid_input_errors() {
        assert!(parse_since("abc").is_err());
        assert!(parse_since("").is_err());
        assert!(parse_since("0h").is_err());
        assert!(parse_since("-1h").is_err());
        assert!(parse_since("5x").is_err());
    }
}

pub fn csv_to_vec(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect()
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
}

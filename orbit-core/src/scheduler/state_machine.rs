use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use orbit_types::{OrbitError, SchedulerRetryBackoffStrategy};

pub fn compute_next_run_at(
    schedule: &str,
    from_utc: DateTime<Utc>,
) -> Result<DateTime<Utc>, OrbitError> {
    let trimmed = schedule.trim();
    if let Some(duration) = parse_interval_alias(trimmed)? {
        return Ok(from_utc + duration);
    }

    compute_next_cron_utc(trimmed, from_utc)
}

pub fn compute_retry_delay_seconds(
    strategy: SchedulerRetryBackoffStrategy,
    initial_delay_seconds: u64,
    retry_index: u32,
) -> u64 {
    if initial_delay_seconds == 0 {
        return 0;
    }

    match strategy {
        SchedulerRetryBackoffStrategy::None => 0,
        SchedulerRetryBackoffStrategy::Fixed => initial_delay_seconds,
        SchedulerRetryBackoffStrategy::Exponential => {
            let exponent = retry_index.saturating_sub(1);
            initial_delay_seconds.saturating_mul(2u64.saturating_pow(exponent))
        }
    }
}

fn parse_interval_alias(spec: &str) -> Result<Option<Duration>, OrbitError> {
    let duration = match spec {
        "@hourly" => Some(Duration::hours(1)),
        "@daily" => Some(Duration::days(1)),
        "@weekly" => Some(Duration::weeks(1)),
        "@monthly" => Some(Duration::days(30)),
        "@yearly" => Some(Duration::days(365)),
        _ => parse_every_alias(spec)?,
    };
    Ok(duration)
}

fn parse_every_alias(spec: &str) -> Result<Option<Duration>, OrbitError> {
    let normalized = spec.trim().to_ascii_lowercase();
    let Some(rest) = normalized.strip_prefix("every ") else {
        return Ok(None);
    };
    if rest.len() < 2 {
        return Err(OrbitError::SchedulerValidation(format!(
            "invalid interval alias: {spec}"
        )));
    }

    let (count_raw, unit_raw) = rest.split_at(rest.len() - 1);
    let count = count_raw.parse::<i64>().map_err(|_| {
        OrbitError::SchedulerValidation(format!("invalid interval count in schedule: {spec}"))
    })?;
    if count <= 0 {
        return Err(OrbitError::SchedulerValidation(
            "interval count must be positive".to_string(),
        ));
    }

    let duration = match unit_raw {
        "s" => Duration::seconds(count),
        "m" => Duration::minutes(count),
        "h" => Duration::hours(count),
        "d" => Duration::days(count),
        "w" => Duration::weeks(count),
        _ => {
            return Err(OrbitError::SchedulerValidation(format!(
                "unsupported interval unit in schedule: {spec}"
            )));
        }
    };
    Ok(Some(duration))
}

fn compute_next_cron_utc(spec: &str, from_utc: DateTime<Utc>) -> Result<DateTime<Utc>, OrbitError> {
    let mut fields = spec.split_whitespace();
    let minute = fields.next();
    let hour = fields.next();
    let day = fields.next();
    let month = fields.next();
    let weekday = fields.next();

    if minute.is_none()
        || hour.is_none()
        || day.is_none()
        || month.is_none()
        || weekday.is_none()
        || fields.next().is_some()
    {
        return Err(OrbitError::SchedulerValidation(format!(
            "invalid cron expression (expected 5 fields): {spec}"
        )));
    }

    let minute = parse_cron_field(minute.unwrap_or_default(), 0, 59, "minute")?;
    let hour = parse_cron_field(hour.unwrap_or_default(), 0, 23, "hour")?;
    let day = parse_cron_field(day.unwrap_or_default(), 1, 31, "day-of-month")?;
    let month = parse_cron_field(month.unwrap_or_default(), 1, 12, "month")?;
    let weekday = parse_cron_field(weekday.unwrap_or_default(), 0, 6, "day-of-week")?;

    let mut candidate = from_utc + Duration::minutes(1);
    candidate = candidate
        .with_second(0)
        .and_then(|d| d.with_nanosecond(0))
        .ok_or_else(|| OrbitError::SchedulerValidation("failed to normalize time".to_string()))?;

    let search_limit_minutes: i64 = 366 * 24 * 60;
    for _ in 0..search_limit_minutes {
        let matches = minute.matches(candidate.minute() as i64)
            && hour.matches(candidate.hour() as i64)
            && day.matches(candidate.day() as i64)
            && month.matches(candidate.month() as i64)
            && weekday.matches(candidate.weekday().num_days_from_sunday() as i64);
        if matches {
            return Ok(candidate);
        }
        candidate += Duration::minutes(1);
    }

    Err(OrbitError::SchedulerValidation(format!(
        "could not compute next run from cron expression: {spec}"
    )))
}

#[derive(Debug, Clone)]
struct CronField {
    any: bool,
    allowed: Vec<i64>,
}

impl CronField {
    fn matches(&self, value: i64) -> bool {
        self.any || self.allowed.contains(&value)
    }
}

fn parse_cron_field(raw: &str, min: i64, max: i64, name: &str) -> Result<CronField, OrbitError> {
    let trimmed = raw.trim();
    if trimmed == "*" {
        return Ok(CronField {
            any: true,
            allowed: vec![],
        });
    }

    let mut allowed = Vec::new();
    for token in trimmed.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(OrbitError::SchedulerValidation(format!(
                "invalid {name} field segment in cron expression"
            )));
        }

        if let Some(step_raw) = token.strip_prefix("*/") {
            let step = parse_i64(step_raw, name)?;
            if step <= 0 {
                return Err(OrbitError::SchedulerValidation(format!(
                    "{name} step must be positive"
                )));
            }
            let mut value = min;
            while value <= max {
                allowed.push(value);
                value += step;
            }
            continue;
        }

        if let Some((start_raw, end_raw)) = token.split_once('-') {
            let start = parse_i64(start_raw, name)?;
            let end = parse_i64(end_raw, name)?;
            if start > end {
                return Err(OrbitError::SchedulerValidation(format!(
                    "{name} range start must be <= end"
                )));
            }
            for value in start..=end {
                ensure_in_range(value, min, max, name)?;
                allowed.push(value);
            }
            continue;
        }

        let value = parse_i64(token, name)?;
        ensure_in_range(value, min, max, name)?;
        allowed.push(value);
    }

    allowed.sort_unstable();
    allowed.dedup();
    Ok(CronField {
        any: false,
        allowed,
    })
}

fn parse_i64(raw: &str, field_name: &str) -> Result<i64, OrbitError> {
    raw.parse::<i64>()
        .map_err(|_| OrbitError::SchedulerValidation(format!("invalid {field_name} value: {raw}")))
}

fn ensure_in_range(value: i64, min: i64, max: i64, field_name: &str) -> Result<(), OrbitError> {
    if value < min || value > max {
        return Err(OrbitError::SchedulerValidation(format!(
            "{field_name} value out of range ({min}-{max}): {value}"
        )));
    }
    Ok(())
}

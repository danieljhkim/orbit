use orbit_core::{AutoTaskSchedule, OrbitError};

/// Resolve the optional `--cron` / `--every-minutes` flags into a schedule
/// patch. On `update`, absent flags mean "leave the schedule unchanged"
/// (`Ok(None)`); supplying both is an error.
pub(super) fn resolve_schedule(
    cron: Option<String>,
    every_minutes: Option<u64>,
) -> Result<Option<AutoTaskSchedule>, OrbitError> {
    match (cron, every_minutes) {
        (Some(_), Some(_)) => Err(OrbitError::InvalidInput(
            "specify exactly one of `--cron` and `--every-minutes`".to_string(),
        )),
        (Some(cron), None) => Ok(Some(AutoTaskSchedule::Cron { cron })),
        (None, Some(every_minutes)) => Ok(Some(AutoTaskSchedule::Interval { every_minutes })),
        (None, None) => Ok(None),
    }
}

/// Resolve a schedule for `add`, where exactly one form is mandatory.
pub(super) fn require_schedule(
    cron: Option<String>,
    every_minutes: Option<u64>,
) -> Result<AutoTaskSchedule, OrbitError> {
    resolve_schedule(cron, every_minutes)?.ok_or_else(|| {
        OrbitError::InvalidInput(
            "a schedule is required: pass one of `--cron` or `--every-minutes`".to_string(),
        )
    })
}

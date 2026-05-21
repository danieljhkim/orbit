use chrono::{DateTime, TimeZone, Utc};

pub(super) fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

mod serde;
mod transitions;
mod validation;

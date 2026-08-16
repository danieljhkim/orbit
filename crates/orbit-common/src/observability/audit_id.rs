use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static AUDIT_EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn audit_execution_id(prefix: &str) -> String {
    let prefix = if prefix.trim().is_empty() {
        "exec"
    } else {
        prefix.trim()
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let sequence = AUDIT_EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{nanos}-{pid}-{sequence}")
}

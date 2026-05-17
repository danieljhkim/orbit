use orbit_common::types::{JobRunState, PipelineState};

pub(crate) fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}

pub(crate) fn format_timestamp(value: Option<chrono::DateTime<chrono::Utc>>) -> String {
    value
        .map(|v| v.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn format_duration(value: Option<u64>) -> String {
    value
        .map(|duration| format!("{duration}ms"))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn format_waiting_line(
    run_state: JobRunState,
    state: Option<&PipelineState>,
) -> Option<String> {
    if run_state.is_terminal() {
        return None;
    }
    let state = state?;
    let deps = state
        .waiting_on_deps
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    let locks = state
        .waiting_on_locks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();

    let mut parts = Vec::new();
    if !deps.is_empty() {
        parts.push(format!("deps: {}", deps.join(", ")));
    }
    if !locks.is_empty() {
        parts.push(format!("locks: {}", locks.join(", ")));
    }
    (!parts.is_empty()).then(|| format!("Waiting on {}", parts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn state_with_waiting(deps: Option<Vec<&str>>, locks: Option<Vec<&str>>) -> PipelineState {
        let mut state =
            PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
        state.set_waiting_reasons(
            deps.map(|values| values.into_iter().map(str::to_string).collect()),
            locks.map(|values| values.into_iter().map(str::to_string).collect()),
        );
        state
    }

    #[test]
    fn waiting_line_lists_deps_and_locks_for_waiting_run() {
        let state = state_with_waiting(Some(vec!["ORB-1", "ORB-2"]), Some(vec!["file:src/lib.rs"]));

        assert_eq!(
            format_waiting_line(JobRunState::Running, Some(&state)),
            Some("Waiting on deps: ORB-1, ORB-2; locks: file:src/lib.rs".to_string())
        );
    }

    #[test]
    fn waiting_line_omits_non_waiting_run() {
        let state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));

        assert_eq!(
            format_waiting_line(JobRunState::Running, Some(&state)),
            None
        );
    }

    #[test]
    fn waiting_line_omits_terminal_run_even_with_stale_reasons() {
        let state = state_with_waiting(Some(vec!["ORB-1"]), Some(vec!["file:src/lib.rs"]));

        assert_eq!(
            format_waiting_line(JobRunState::Success, Some(&state)),
            None
        );
    }
}

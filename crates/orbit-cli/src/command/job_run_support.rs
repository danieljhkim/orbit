use chrono::{DateTime, Duration, Utc};
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::{JobRun, JobRunState, JobRunStep, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
pub(crate) struct RunHistoryFilter {
    pub status: Option<JobRunState>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

pub(crate) fn load_filtered_job_runs(
    runtime: &OrbitRuntime,
    job_ids: &[&str],
    filter: &RunHistoryFilter,
) -> Result<Vec<JobRun>, OrbitError> {
    let since = filter
        .since
        .as_deref()
        .map(crate::parse::parse_duration_seconds)
        .transpose()?
        .map(|seconds| Utc::now() - Duration::seconds(seconds as i64));

    let mut runs = runtime.list_job_runs(JobRunListParams {
        job_id: None,
        state: filter.status,
        since,
        limit: None,
    })?;
    runs.retain(|run| job_ids.contains(&run.job_id.as_str()));
    runs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.run_id.cmp(&left.run_id))
    });
    if let Some(limit) = filter.limit {
        runs.truncate(limit);
    }
    Ok(runs)
}

pub(crate) fn load_latest_job_run(
    runtime: &OrbitRuntime,
    job_ids: &[&str],
    label: &str,
) -> Result<JobRun, OrbitError> {
    load_filtered_job_runs(
        runtime,
        job_ids,
        &RunHistoryFilter {
            limit: Some(1),
            ..RunHistoryFilter::default()
        },
    )?
    .into_iter()
    .next()
    .ok_or_else(|| OrbitError::InvalidInput(format!("no {label} runs found")))
}

pub(crate) fn print_job_run_list(runs: &[JobRun], full: bool) {
    let headers = if full {
        vec![
            "RUN_ID",
            "JOB_ID",
            "ATTEMPT",
            "STATE",
            "STARTED",
            "FINISHED",
            "DURATION",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]
    } else {
        vec!["RUN_ID", "STATE", "STARTED", "FINISHED", "DURATION"]
    };
    let mut table = crate::output::table::build_table(&headers);
    for run in runs {
        use comfy_table::Cell;
        let mut row = vec![
            Cell::new(&run.run_id),
            crate::output::color::job_state_color_cell(&run.state.to_string()),
            Cell::new(format_table_timestamp(run.started_at)),
            Cell::new(format_table_timestamp(run.finished_at)),
            Cell::new(format_run_duration(run)),
        ];

        if full {
            row.insert(1, Cell::new(&run.job_id));
            row.insert(2, Cell::new(run.attempt.to_string()));
            row.extend([
                Cell::new(
                    run.steps
                        .last()
                        .and_then(|step| step.error_code.clone())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::new(summarize_error_message(
                    run.steps
                        .last()
                        .and_then(|step| step.error_message.as_deref()),
                )),
            ]);
        }

        crate::output::table::add_single_line_row(&mut table, row);
    }
    println!("{table}");
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    let last = run.steps.last();
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|value| value.to_rfc3339()),
        "finished_at": run.finished_at.map(|value| value.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "exit_code": last.and_then(|step| step.exit_code),
        "agent_response_json": last.and_then(|step| step.agent_response_json.as_ref()),
        "error_code": last.and_then(|step| step.error_code.as_deref()),
        "error_message": last.and_then(|step| step.error_message.as_deref()),
        "knowledge_metrics": run.knowledge_metrics,
        "steps": run.steps.iter().map(job_run_step_to_json).collect::<Vec<_>>(),
        "created_at": run.created_at.to_rfc3339(),
    })
}

pub(crate) fn job_run_step_to_json(step: &JobRunStep) -> Value {
    json!({
        "step_index": step.step_index,
        "target_type": step.target_type.to_string(),
        "target_id": step.target_id,
        "state": step.state.to_string(),
        "started_at": step.started_at.map(|value| value.to_rfc3339()),
        "finished_at": step.finished_at.map(|value| value.to_rfc3339()),
        "duration_ms": step.duration_ms,
        "exit_code": step.exit_code,
        "agent_response_json": step.agent_response_json,
        "error_code": step.error_code,
        "error_message": step.error_message,
    })
}

pub(crate) fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}

fn format_table_timestamp(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_run_duration(run: &JobRun) -> String {
    format_run_duration_values(run.started_at, run.finished_at)
}

fn format_run_duration_values(
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
) -> String {
    match (started_at, finished_at) {
        (Some(started_at), Some(finished_at)) if finished_at >= started_at => {
            format_duration(finished_at - started_at)
        }
        _ => "-".to_string(),
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.num_seconds();
    if seconds < 0 {
        return "-".to_string();
    }

    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        if hours > 0 {
            return format!("{days}d{hours}h");
        }
        return format!("{days}d");
    }

    if hours > 0 {
        if minutes > 0 {
            return format!("{hours}h{minutes}m");
        }
        return format!("{hours}h");
    }

    if minutes > 0 {
        if secs > 0 {
            return format!("{minutes}m{secs}s");
        }
        return format!("{minutes}m");
    }

    format!("{secs}s")
}

pub(crate) fn print_job_run(run: &JobRun) {
    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Run ID:"), run.run_id);
    println!("{} {}", bold("Job ID:"), run.job_id);
    println!("{} {}", bold("Attempt:"), run.attempt);
    println!(
        "{} {}",
        bold("State:"),
        job_state_color(&run.state.to_string())
    );
    println!(
        "{} {}",
        bold("Scheduled:"),
        dimmed(&run.scheduled_at.to_rfc3339())
    );
    println!(
        "{} {}",
        bold("Started:"),
        run.started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Finished:"),
        run.finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Duration (ms):"),
        run.duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Created:"),
        dimmed(&run.created_at.to_rfc3339())
    );

    if run.steps.is_empty() {
        println!("\n{}", bold("Steps: (none)"));
        return;
    }

    println!("\n{}", bold("Steps:"));
    let mut table = crate::output::table::build_table(&[
        "STEP",
        "TARGET_ID",
        "STATE",
        "DURATION_MS",
        "EXIT_CODE",
        "ERROR_CODE",
        "ERROR_MESSAGE",
    ]);
    for step in &run.steps {
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(step.step_index.to_string()),
            Cell::new(&step.target_id),
            crate::output::color::job_state_color_cell(&step.state.to_string()),
            Cell::new(
                step.duration_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::new(
                step.exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::new(step.error_code.as_deref().unwrap_or("-")),
            Cell::new(summarize_error_message(step.error_message.as_deref())),
        ]);
    }
    println!("{table}");
    println!(
        "  {}",
        dimmed("Use --step <n> to inspect full details for a step.")
    );
}

pub(crate) fn print_step_detail(step: &JobRunStep) {
    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Step Index:"), step.step_index);
    println!("{} {}", bold("Target Type:"), step.target_type);
    println!("{} {}", bold("Target ID:"), step.target_id);
    println!(
        "{} {}",
        bold("State:"),
        job_state_color(&step.state.to_string())
    );
    println!(
        "{} {}",
        bold("Started:"),
        step.started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Finished:"),
        step.finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Duration (ms):"),
        step.duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Exit Code:"),
        step.exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Error Code:"),
        step.error_code.as_deref().unwrap_or("-")
    );
    println!(
        "{} {}",
        bold("Error Message:"),
        step.error_message.as_deref().unwrap_or("-")
    );
    if let Some(response) = &step.agent_response_json {
        let rendered =
            serde_json::to_string_pretty(response).unwrap_or_else(|_| "<invalid-json>".to_string());
        println!("{}", bold("Agent Response:"));
        for line in rendered.lines() {
            println!("  {}", dimmed(line));
        }
    } else {
        println!("{} -", bold("Agent Response:"));
    }
}

#[cfg(test)]
mod tests {
    use super::{format_duration, format_run_duration_values, format_table_timestamp};
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn table_timestamp_is_shortened() {
        let value = Utc.with_ymd_and_hms(2026, 4, 11, 18, 45, 12).unwrap();
        assert_eq!(format_table_timestamp(Some(value)), "2026-04-11 18:45");
    }

    #[test]
    fn human_duration_prefers_large_units() {
        assert_eq!(format_duration(Duration::seconds(59)), "59s");
        assert_eq!(format_duration(Duration::minutes(30)), "30m");
        assert_eq!(format_duration(Duration::minutes(72)), "1h12m");
        assert_eq!(format_duration(Duration::hours(27)), "1d3h");
    }

    #[test]
    fn run_duration_uses_start_and_finish() {
        let started_at = Utc.with_ymd_and_hms(2026, 4, 11, 18, 0, 0).unwrap();
        let finished_at = Utc.with_ymd_and_hms(2026, 4, 11, 19, 12, 0).unwrap();

        assert_eq!(
            format_run_duration_values(Some(started_at), Some(finished_at)),
            "1h12m"
        );
    }
}

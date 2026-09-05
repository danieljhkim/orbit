use chrono::{DateTime, TimeZone, Utc};
use orbit_types::workflow::{JobRun, JobRunState, JobRunStep, JobTargetType};

use crate::Store;
use crate::contracts::{JobRunOrder, JobRunQuery};

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 0, minute, 0)
        .single()
        .expect("valid timestamp")
}

fn run_with_steps(run_id: &str, state: JobRunState, created: DateTime<Utc>, steps: u32) -> JobRun {
    JobRun {
        run_id: run_id.to_string(),
        job_id: "task_pr_pipeline".to_string(),
        attempt: 1,
        state,
        scheduled_at: created,
        started_at: Some(created),
        finished_at: state.is_terminal().then_some(created),
        duration_ms: state.is_terminal().then_some(u64::from(steps) * 100),
        created_at: created,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: (0..steps)
            .map(|index| JobRunStep {
                step_index: index,
                target_type: JobTargetType::Activity,
                target_id: format!("step-{index}"),
                state,
                started_at: Some(created),
                finished_at: Some(created),
                duration_ms: Some(100),
                exit_code: Some(0),
                error_code: None,
                error_message: None,
                agent_response_json: None,
            })
            .collect(),
    }
}

/// The run upsert persists the run row only; steps are written per step.
fn insert_run_with_steps(store: &Store, workspace_id: &str, run: &JobRun) {
    store
        .upsert_job_run_for_workspace(workspace_id, run, None)
        .expect("insert run");
    for step in &run.steps {
        store
            .upsert_job_run_step_for_workspace(workspace_id, &run.run_id, step)
            .expect("insert step");
    }
}

/// Steps come back for every run on a page whatever its size: the page is
/// hydrated with one query per id chunk, not one per run, so a long history
/// must not lose or cross-wire any run's steps.
#[test]
fn listing_hydrates_every_runs_steps_across_id_chunks() {
    let store = Store::open_in_memory().expect("open store");
    let total = 1_100_u32;
    for index in 0..total {
        let run = run_with_steps(
            &format!("jrun-{index:05}"),
            JobRunState::Success,
            at(index % 60),
            index % 4,
        );
        insert_run_with_steps(&store, "ws", &run);
    }

    let runs = store
        .list_job_runs_for_workspace("ws", &JobRunQuery::default())
        .expect("list runs");
    assert_eq!(runs.len(), total as usize);
    for run in &runs {
        let index: u32 = run.run_id["jrun-".len()..].parse().expect("fixture id");
        assert_eq!(run.steps.len(), (index % 4) as usize, "{}", run.run_id);
        for (position, step) in run.steps.iter().enumerate() {
            assert_eq!(step.step_index, position as u32, "{}", run.run_id);
            assert_eq!(step.target_id, format!("step-{position}"), "{}", run.run_id);
        }
    }
}

/// Counting and duration reads apply the list filter but ignore its limit.
#[test]
fn count_and_durations_ignore_the_page_limit() {
    let store = Store::open_in_memory().expect("open store");
    for index in 0..30_u32 {
        let state = if index % 3 == 0 {
            JobRunState::Failed
        } else if index % 3 == 1 {
            JobRunState::Success
        } else {
            JobRunState::Running
        };
        let run = run_with_steps(&format!("jrun-{index:03}"), state, at(index), 1 + index % 2);
        insert_run_with_steps(&store, "ws", &run);
    }
    // Another workspace's rows must not leak into the count.
    let other = run_with_steps("jrun-other", JobRunState::Failed, at(5), 1);
    store
        .upsert_job_run_for_workspace("other-ws", &other, None)
        .expect("insert other run");

    let failed = JobRunQuery {
        state: Some(JobRunState::Failed),
        limit: Some(2),
        ..JobRunQuery::default()
    };
    assert_eq!(
        store
            .list_job_runs_for_workspace("ws", &failed)
            .expect("page")
            .len(),
        2
    );
    assert_eq!(
        store
            .count_job_runs_for_workspace("ws", &failed)
            .expect("count"),
        10
    );

    let terminal_since = JobRunQuery {
        terminal_only: true,
        created_since: Some(at(15)),
        limit: Some(1),
        ..JobRunQuery::default()
    };
    let mut durations = store
        .list_job_run_durations_for_workspace("ws", &terminal_since)
        .expect("durations");
    durations.sort_unstable();
    // Terminal runs created at minute >= 15: indexes 15..30 with index % 3 != 2
    // → 10 runs; durations are 100 * (1 + index % 2).
    assert_eq!(durations.len(), 10);
    assert!(durations.iter().all(|d| *d == 100 || *d == 200));
    assert_eq!(
        store
            .count_job_runs_for_workspace("ws", &terminal_since)
            .expect("count"),
        10
    );
}

/// [ORB-11251] `CreatedAt` orders and truncates by `created_at`, so a run
/// created earlier but still running the longest can be limited away before
/// its later `finished_at` is ever considered. `Recency` orders and
/// truncates by `finished_at`/`started_at`/`created_at` instead, so the same
/// bounded query surfaces the run that most recently finished — the
/// ordering the dashboard displays — rather than dropping it under `LIMIT`.
#[test]
fn recency_order_truncates_by_finish_time_not_creation_time() {
    let store = Store::open_in_memory().expect("open store");
    let mut old_long_running =
        run_with_steps("jrun-old-long-running", JobRunState::Success, at(0), 1);
    old_long_running.finished_at = Some(at(50));
    let mut new_short_running =
        run_with_steps("jrun-new-short-running", JobRunState::Success, at(10), 1);
    new_short_running.finished_at = Some(at(20));
    insert_run_with_steps(&store, "ws", &old_long_running);
    insert_run_with_steps(&store, "ws", &new_short_running);

    let by_created_at = JobRunQuery {
        limit: Some(1),
        ..JobRunQuery::default()
    };
    let top_by_created_at = store
        .list_job_runs_for_workspace("ws", &by_created_at)
        .expect("list by created_at");
    assert_eq!(
        top_by_created_at[0].run_id, "jrun-new-short-running",
        "created_at DESC ranks the newer-created run first, even though it finished earlier"
    );

    let by_recency = JobRunQuery {
        limit: Some(1),
        order_by: JobRunOrder::Recency,
        ..JobRunQuery::default()
    };
    let top_by_recency = store
        .list_job_runs_for_workspace("ws", &by_recency)
        .expect("list by recency");
    assert_eq!(
        top_by_recency[0].run_id, "jrun-old-long-running",
        "recency ordering must select the most-recently-finished run before LIMIT applies"
    );
}

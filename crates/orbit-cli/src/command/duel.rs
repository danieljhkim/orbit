//! `orbit run duel` CLI subcommand tree.
//!
//! Thin presentation layer over `orbit_core::duel_scoreboard::aggregate`.
//! All math lives in the store crate (re-exported via orbit-core) so tests
//! and future programmatic callers can reach the same numbers without
//! reimplementing anything.

use crate::command::Execute;
use crate::command::job_run_support::{
    RunHistoryFilter, job_run_step_to_json, job_run_to_json_with_workflow,
    print_job_run_list_with_workflow, print_job_run_with_workflow, print_step_detail,
};
use chrono::Utc;
use clap::{Args, Subcommand, ValueEnum};
use orbit_common::types::DuelRun;
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::duel_scoreboard::{
    AggregateFilter, AggregateRow, Aggregates, RoleAxis, SegmentBy, aggregate,
};
use orbit_core::{OrbitError, OrbitRuntime};

const DUEL_PR_WORKFLOW: &str = "duel";
const DUEL_PLAN_WORKFLOW: &str = "duel-plan";
const DUEL_JOB_IDS: &[&str] = &["job_duel_pipeline", "job_duel_plan_pipeline"];

#[derive(Args)]
#[command(
    about = "Inspect cross-agent duel history and scoreboards",
    override_usage = "orbit run duel [OPTIONS]\n       orbit run duel <COMMAND>\n       orbit run duel <TASK_ID> [RETIRED_OPTIONS]"
)]
pub struct DuelCommand {
    #[command(subcommand)]
    pub command: Option<DuelSubcommand>,

    #[command(flatten)]
    pub direct: DuelPrArgs,
}

impl Execute for DuelCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            Some(command) => command.execute(runtime),
            None if self.direct.defaults_to_scoreboard() => DuelScoreboardArgs {
                by: SegmentByArg::None,
                role: RoleFilterArg::All,
                json: self.direct.json,
            }
            .execute(runtime),
            None => self.direct.execute(runtime),
        }
    }
}

impl DuelCommand {
    pub(crate) fn defaults_to_scoreboard(&self) -> bool {
        self.command.is_none() && self.direct.defaults_to_scoreboard()
    }
}

#[derive(Subcommand)]
pub enum DuelSubcommand {
    /// Run a single-task PR duel through the legacy v1 job runtime
    #[command(hide = true)]
    Pr(DuelPrArgs),
    /// Run a single-task planning duel through the legacy v1 job runtime
    Plan(DuelPlanArgs),
    /// Show scoreboard aggregates computed from `.orbit/state/scoreboard/duel.json`.
    #[command(alias = "scoreboard")]
    Score(DuelScoreboardArgs),
    /// List duel job runs
    List(DuelListArgs),
    /// Show a duel run, or the latest one when no run ID is provided
    Show(DuelShowArgs),
}

impl Execute for DuelSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            DuelSubcommand::Pr(args) => args.execute(runtime),
            DuelSubcommand::Plan(args) => args.execute(runtime),
            DuelSubcommand::Score(args) => args.execute(runtime),
            DuelSubcommand::List(args) => args.execute(runtime),
            DuelSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Read-only examples:\n  orbit run duel\n  orbit run duel --json\n  orbit run duel score --by scope\n  orbit run duel list\n  orbit run duel show\n\nRetired execution forms like `orbit run duel T20260409-0310` still parse so Orbit can return an explicit retirement error."
)]
pub struct DuelPrArgs {
    /// Retired execution arg. Supplying a task ID now returns the duel retirement error.
    #[arg(value_name = "TASK_ID", num_args = 0..=1)]
    pub task_id: Option<String>,
    /// Retired execution arg. Preserved only so Orbit can emit the retirement error.
    #[arg(long)]
    pub base: Option<String>,
    /// Retired execution arg. Preserved only so Orbit can emit the retirement error.
    #[arg(long)]
    pub debug: bool,
    /// For bare `orbit run duel`, emit the scoreboard alias as JSON.
    #[arg(long)]
    pub json: bool,
    /// Retired execution arg. Preserved only so Orbit can emit the retirement error.
    #[arg(long = "loop")]
    pub loop_count: Option<u32>,
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit run duel plan T20260409-0310\n  orbit run duel plan T20260409-0310 --base main --json"
)]
pub struct DuelPlanArgs {
    /// Task ID for the planning duel.
    pub task_id: String,
    /// Base branch for the planning duel pipeline
    #[arg(long)]
    pub base: Option<String>,
    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelPrArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let _ = runtime;
        Err(OrbitError::InvalidInput(
            "the legacy duel execution workflows were retired in T20260419-2156; use the preserved scoreboard/list/show surfaces for historical results, or reopen duel support as a new feature task".to_string(),
        ))
    }
}

impl DuelPrArgs {
    fn defaults_to_scoreboard(&self) -> bool {
        self.task_id.is_none() && self.base.is_none() && !self.debug && self.loop_count.is_none()
    }
}

impl Execute for DuelPlanArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let _ = (self, runtime);
        Err(OrbitError::InvalidInput(
            "the legacy planning duel workflow was retired in T20260419-2156; reopen planning-duel support as a new feature task if it is still needed".to_string(),
        ))
    }
}

/// How the flat table should be sliced before display.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SegmentByArg {
    /// No segmentation — one row per (role, agent, model).
    None,
    /// Segment by `task_class.scope` (single_file / multi_file / cross_crate / other).
    Scope,
    /// Segment by `task_class.ambiguity` (well_specified / needs_judgment / exploratory / unknown).
    Ambiguity,
}

impl From<SegmentByArg> for SegmentBy {
    fn from(value: SegmentByArg) -> Self {
        match value {
            SegmentByArg::None => SegmentBy::None,
            SegmentByArg::Scope => SegmentBy::Scope,
            SegmentByArg::Ambiguity => SegmentBy::Ambiguity,
        }
    }
}

/// Role filter — mirrors [`RoleAxis`] but adds `All` as the default.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum RoleFilterArg {
    /// All three roles (default).
    All,
    Implementer,
    Reviewer,
    Arbiter,
}

impl RoleFilterArg {
    fn into_filter(self) -> Option<RoleAxis> {
        match self {
            RoleFilterArg::All => None,
            RoleFilterArg::Implementer => Some(RoleAxis::Implementer),
            RoleFilterArg::Reviewer => Some(RoleAxis::Reviewer),
            RoleFilterArg::Arbiter => Some(RoleAxis::Arbiter),
        }
    }
}

#[derive(Args)]
pub struct DuelScoreboardArgs {
    /// Segment the table by a `task_class` dimension.
    #[arg(long, value_enum, default_value_t = SegmentByArg::None)]
    pub by: SegmentByArg,
    /// Filter to a single role. Defaults to showing all three roles.
    #[arg(long, value_enum, default_value_t = RoleFilterArg::All)]
    pub role: RoleFilterArg,
    /// Emit raw aggregates as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelScoreboardArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = runtime.load_duel_runs()?;
        let filter = AggregateFilter {
            segment_by: self.by.into(),
            role: self.role.into_filter(),
        };
        let aggs = aggregate(&runs, filter);

        if self.json {
            return emit_json(&runs, &aggs);
        }
        render_table(&runs, &aggs);
        Ok(())
    }
}

#[derive(Args)]
pub struct DuelListArgs {
    #[arg(long, value_enum)]
    pub status: Option<orbit_core::JobRunState>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub full: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = load_filtered_stored_duel_runs(
            runtime,
            &RunHistoryFilter {
                status: self.status,
                since: self.since,
                limit: self.limit,
            },
        )?;

        if self.json {
            return crate::output::json::print_pretty(&serde_json::Value::Array(
                runs.iter()
                    .map(|run| {
                        job_run_to_json_with_workflow(run, duel_workflow_name(run.job_id.as_str()))
                    })
                    .collect::<Vec<_>>(),
            ));
        }
        if runs.is_empty() {
            println!("No duel runs recorded yet.");
            return Ok(());
        }

        print_job_run_list_with_workflow(&runs, self.full, duel_workflow_name);
        Ok(())
    }
}

#[derive(Args)]
pub struct DuelShowArgs {
    pub run_id: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub step: Option<usize>,
}

impl Execute for DuelShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = match &self.run_id {
            Some(run_id) => runtime.show_job_run(run_id)?,
            None => load_latest_stored_duel_run(runtime)?,
        };
        ensure_duel_run(&run)?;

        if let Some(step_index) = self.step {
            let step = run
                .steps
                .iter()
                .find(|step| step.step_index as usize == step_index)
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "step {step_index} not found in run '{}' (run has {} step(s))",
                        run.run_id,
                        run.steps.len()
                    ))
                })?;
            if self.json {
                return crate::output::json::print_pretty(&job_run_step_to_json(step));
            }
            print_step_detail(step);
            return Ok(());
        }

        if self.json {
            return crate::output::json::print_pretty(&job_run_to_json_with_workflow(
                &run,
                duel_workflow_name(run.job_id.as_str()),
            ));
        }

        print_job_run_with_workflow(&run, duel_workflow_name(run.job_id.as_str()));
        Ok(())
    }
}

fn emit_json(runs: &[DuelRun], aggs: &Aggregates) -> Result<(), OrbitError> {
    let payload = serde_json::json!({
        "runs": runs.len(),
        "rows": aggs.rows.iter().map(row_to_json).collect::<Vec<_>>(),
    });
    crate::output::json::print_pretty(&payload)
}

fn row_to_json(row: &AggregateRow) -> serde_json::Value {
    serde_json::json!({
        "segment": row.segment,
        "role": row.role,
        "agent": row.agent,
        "model": row.model,
        "runs": row.runs,
        "avg_score": row.avg_score,
        "merge_rate": row.merge_rate,
        "avg_fix_iterations": row.avg_fix_iterations,
        "avg_wall_seconds": row.avg_wall_seconds,
    })
}

fn render_table(runs: &[DuelRun], aggs: &Aggregates) {
    if runs.is_empty() {
        println!("No duel runs recorded yet.");
        return;
    }
    if aggs.rows.is_empty() {
        println!("No rows match the selected filters (runs={}).", runs.len());
        return;
    }

    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "SEGMENT",
        "ROLE",
        "AGENT/MODEL",
        "RUNS",
        "AVG SCORE",
        "MERGE RATE",
        "AVG FIX ITERS",
        "AVG WALL SECS",
    ]);
    for row in &aggs.rows {
        table.add_row(vec![
            Cell::new(&row.segment),
            Cell::new(row.role),
            Cell::new(format!("{} / {}", row.agent, row.model)),
            Cell::new(row.runs),
            Cell::new(format!("{:.2}", row.avg_score)),
            Cell::new(format!("{:.2}", row.merge_rate)),
            Cell::new(format!("{:.2}", row.avg_fix_iterations)),
            Cell::new(format!("{:.0}", row.avg_wall_seconds)),
        ]);
    }
    println!("{table}");
}

fn ensure_duel_run(run: &orbit_core::JobRun) -> Result<(), OrbitError> {
    if DUEL_JOB_IDS.contains(&run.job_id.as_str()) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "run '{}' belongs to job '{}', not a duel pipeline",
        run.run_id, run.job_id
    )))
}

fn duel_workflow_name(job_id: &str) -> Option<&'static str> {
    match job_id {
        "job_duel_pipeline" => Some(DUEL_PR_WORKFLOW),
        "job_duel_plan_pipeline" => Some(DUEL_PLAN_WORKFLOW),
        _ => None,
    }
}

fn load_filtered_stored_duel_runs(
    runtime: &OrbitRuntime,
    filter: &RunHistoryFilter,
) -> Result<Vec<orbit_core::JobRun>, OrbitError> {
    let since = filter
        .since
        .as_deref()
        .map(crate::parse::parse_since)
        .transpose()?
        .map(|value| value.with_timezone(&Utc));

    let mut runs = runtime.list_job_runs(JobRunListParams {
        job_id: None,
        state: filter.status,
        since,
        limit: None,
    })?;
    runs.retain(|run| DUEL_JOB_IDS.contains(&run.job_id.as_str()));
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

fn load_latest_stored_duel_run(runtime: &OrbitRuntime) -> Result<orbit_core::JobRun, OrbitError> {
    load_filtered_stored_duel_runs(
        runtime,
        &RunHistoryFilter {
            limit: Some(1),
            ..RunHistoryFilter::default()
        },
    )?
    .into_iter()
    .next()
    .ok_or_else(|| OrbitError::InvalidInput("no duel runs found".to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use orbit_core::{JobRun, JobRunState, JobRunStep, JobTargetType};
    use serde::Serialize;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct JobRunFileDocument<'a> {
        schema_version: u8,
        run: &'a JobRun,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct JobRunStepFileDocument<'a> {
        schema_version: u8,
        step: &'a JobRunStep,
    }

    fn test_runtime() -> (TempDir, OrbitRuntime, PathBuf) {
        let root = tempfile::tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let workspace_root = root.path().join("repo").join(".orbit");
        fs::create_dir_all(&global_root).expect("create global root");
        fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, workspace_root)
    }

    fn write_run_bundle(
        orbit_root: &Path,
        run: &JobRun,
        steps: &[JobRunStep],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let run_dir = orbit_root
            .join("state")
            .join("job-runs")
            .join(&run.job_id)
            .join(&run.run_id);
        fs::create_dir_all(run_dir.join("steps"))?;
        let jrun = JobRunFileDocument {
            schema_version: 1,
            run,
        };
        fs::write(run_dir.join("jrun.yaml"), serde_yaml::to_string(&jrun)?)?;
        for step in steps {
            let step_doc = JobRunStepFileDocument {
                schema_version: 1,
                step,
            };
            let path = run_dir.join("steps").join(format!(
                "{:02}-{}.yaml",
                step.step_index + 1,
                step.target_id
            ));
            fs::write(path, serde_yaml::to_string(&step_doc)?)?;
        }
        Ok(())
    }

    fn sample_run(run_id: &str, job_id: &str, created_at: &str) -> JobRun {
        JobRun {
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            attempt: 1,
            state: JobRunState::Success,
            scheduled_at: created_at.parse().expect("scheduled_at"),
            started_at: Some(created_at.parse().expect("started_at")),
            finished_at: Some(created_at.parse().expect("finished_at")),
            duration_ms: Some(1_000),
            created_at: created_at.parse().expect("created_at"),
            pid: None,
            pid_start_time: None,
            input: Some(json!({ "task_id": "T20260423-0447" })),
            retry_source_run_id: None,
            knowledge_metrics: None,
            steps: Vec::new(),
        }
    }

    fn sample_step(step_index: u32) -> JobRunStep {
        JobRunStep {
            step_index,
            target_type: JobTargetType::Activity,
            target_id: "record_duel_scores".to_string(),
            started_at: Some("2026-04-23T04:00:00Z".parse().expect("step started_at")),
            finished_at: Some("2026-04-23T04:00:01Z".parse().expect("step finished_at")),
            duration_ms: Some(1_000),
            exit_code: Some(0),
            agent_response_json: Some(json!({ "decision": "APPROVED" })),
            state: JobRunState::Success,
            error_code: None,
            error_message: None,
        }
    }

    #[test]
    fn bare_duel_defaults_to_scoreboard() {
        let (_root, runtime, _orbit_root) = test_runtime();
        let command = DuelCommand {
            command: None,
            direct: DuelPrArgs {
                task_id: None,
                base: None,
                debug: false,
                json: false,
                loop_count: None,
            },
        };

        assert!(command.defaults_to_scoreboard());
        command
            .execute(&runtime)
            .expect("bare duel aliases to score");
    }

    #[test]
    fn explicit_duel_execution_request_still_errors() {
        let (_root, runtime, _orbit_root) = test_runtime();
        let command = DuelCommand {
            command: None,
            direct: DuelPrArgs {
                task_id: Some("T20260423-0447".to_string()),
                base: None,
                debug: false,
                json: false,
                loop_count: None,
            },
        };

        let err = command
            .execute(&runtime)
            .expect_err("retired duel execution");
        assert!(
            err.to_string()
                .contains("legacy duel execution workflows were retired"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn duel_list_and_latest_show_work_without_live_job_assets() {
        let (_root, runtime, orbit_root) = test_runtime();
        let duel_run = sample_run(
            "jrun-20260423-0410",
            "job_duel_pipeline",
            "2026-04-23T04:10:00Z",
        );
        let later_duel_plan = sample_run(
            "jrun-20260423-0420",
            "job_duel_plan_pipeline",
            "2026-04-23T04:20:00Z",
        );
        let non_duel = sample_run(
            "jrun-20260423-0430",
            "task_local_pipeline",
            "2026-04-23T04:30:00Z",
        );
        write_run_bundle(&orbit_root, &duel_run, &[sample_step(0)]).expect("write duel run");
        write_run_bundle(&orbit_root, &later_duel_plan, &[sample_step(0)])
            .expect("write duel-plan run");
        write_run_bundle(&orbit_root, &non_duel, &[sample_step(0)]).expect("write non-duel run");

        let runs = load_filtered_stored_duel_runs(&runtime, &RunHistoryFilter::default())
            .expect("load stored duel runs");
        assert_eq!(
            runs.iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["jrun-20260423-0420", "jrun-20260423-0410"]
        );
        assert_eq!(
            load_latest_stored_duel_run(&runtime)
                .expect("latest duel run")
                .run_id,
            "jrun-20260423-0420"
        );

        DuelListArgs {
            status: None,
            since: None,
            limit: None,
            full: false,
            json: false,
        }
        .execute(&runtime)
        .expect("list should work without live job assets");

        DuelShowArgs {
            run_id: None,
            json: false,
            step: None,
        }
        .execute(&runtime)
        .expect("latest show should work without live job assets");
    }

    #[test]
    fn duel_show_rejects_non_duel_run() {
        let (_root, runtime, orbit_root) = test_runtime();
        let non_duel = sample_run(
            "jrun-20260423-0430",
            "task_local_pipeline",
            "2026-04-23T04:30:00Z",
        );
        write_run_bundle(&orbit_root, &non_duel, &[sample_step(0)]).expect("write non-duel run");

        let err = DuelShowArgs {
            run_id: Some("jrun-20260423-0430".to_string()),
            json: false,
            step: None,
        }
        .execute(&runtime)
        .expect_err("non-duel runs must be rejected");
        assert!(
            err.to_string().contains("not a duel pipeline"),
            "unexpected error: {err}"
        );
    }
}

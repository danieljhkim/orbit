//! `orbit duel` CLI subcommand tree.
//!
//! Thin presentation layer over `orbit_core::duel_scoreboard::aggregate`.
//! All math lives in the store crate (re-exported via orbit-core) so tests
//! and future programmatic callers can reach the same numbers without
//! reimplementing anything.

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::duel_scoreboard::{
    AggregateFilter, AggregateRow, Aggregates, RoleAxis, SegmentBy, aggregate,
};
use orbit_core::{
    OrbitError, OrbitRuntime, WorkflowInput, build_workflow_input_for, find_workflow,
    validate_workflow_flags,
};
use orbit_types::DuelRun;
use serde_json::json;

use crate::command::Execute;
use crate::command::job_run_support::{
    RunHistoryFilter, job_run_step_to_json, job_run_to_json, load_filtered_job_runs,
    load_latest_job_run, print_job_run, print_job_run_list, print_step_detail,
};

const DUEL_WORKFLOW: &str = "duel";
const DUEL_JOB_IDS: &[&str] = &["job_duel_pipeline"];

#[derive(Args)]
#[command(
    about = "Cross-agent scoring",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct DuelCommand {
    #[command(subcommand)]
    pub command: DuelSubcommand,
}

impl Execute for DuelCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum DuelSubcommand {
    /// Run a single-task duel
    Run(DuelRunArgs),
    /// Show scoreboard aggregates computed from `.orbit/scoreboard/duel.json`.
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
            DuelSubcommand::Run(args) => args.execute(runtime),
            DuelSubcommand::Score(args) => args.execute(runtime),
            DuelSubcommand::List(args) => args.execute(runtime),
            DuelSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit duel run\n  orbit duel run T20260409-0310\n  orbit duel run T20260409-0310 --base main --json"
)]
pub struct DuelRunArgs {
    /// Optional task ID. Omit to auto-select the first available duel-eligible task.
    pub task_id: Option<String>,
    /// Base branch for the duel pipeline
    #[arg(long)]
    pub base: Option<String>,
    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let workflow = find_workflow(DUEL_WORKFLOW)
            .ok_or_else(|| OrbitError::InvalidInput("unknown workflow 'duel'".to_string()))?;
        let input = WorkflowInput {
            tasks: self.task_id.clone(),
            parallelism: None,
            base: self.base.clone(),
            pr_number: None,
        };
        validate_workflow_flags(workflow, &input)?;
        let built_input = build_workflow_input_for(Some(workflow), &input)?;
        let run = runtime.run_job_now_with_input_debug(workflow.job_id, built_input, self.debug)?;
        let run_details = runtime
            .job_history(workflow.job_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);

        if self.json {
            return crate::output::json::print_pretty(&json!({
                "workflow": workflow.alias,
                "job_id": run.job_id,
                "run_id": run.run_id,
                "state": run.state.to_string(),
                "attempt": run.attempt,
                "error_code": run_details.as_ref().and_then(|entry| entry.steps.last()).and_then(|step| step.error_code.clone()),
                "error_message": run_details.as_ref().and_then(|entry| entry.steps.last()).and_then(|step| step.error_message.clone()),
            }));
        }

        let error_code = run_details
            .as_ref()
            .and_then(|entry| entry.steps.last())
            .and_then(|step| step.error_code.clone())
            .unwrap_or_else(|| "-".to_string());
        let error_message = run_details
            .as_ref()
            .and_then(|entry| entry.steps.last())
            .and_then(|step| step.error_message.clone())
            .unwrap_or_else(|| "-".to_string())
            .replace('\n', " ");
        println!(
            "workflow={};job_id={};run_id={};state={};attempt={};error_code={};error_message={}",
            workflow.alias,
            run.job_id,
            run.run_id,
            run.state,
            run.attempt,
            error_code,
            error_message
        );
        Ok(())
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
        let runs = load_filtered_job_runs(
            runtime,
            DUEL_JOB_IDS,
            &RunHistoryFilter {
                status: self.status,
                since: self.since,
                limit: self.limit,
            },
        )?;

        if self.json {
            return crate::output::json::print_pretty(&serde_json::Value::Array(
                runs.iter().map(job_run_to_json).collect::<Vec<_>>(),
            ));
        }

        print_job_run_list(&runs, self.full);
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
            None => load_latest_job_run(runtime, DUEL_JOB_IDS, "duel")?,
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
            return crate::output::json::print_pretty(&job_run_to_json(&run));
        }

        print_job_run(&run);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use clap::Parser;
    use orbit_types::{
        Ambiguity, Cost, Decision, DuelRun, ImplementerStats, Outcome, ReviewerStats,
        RoleAssignment, Roles, Scores, TaskClass, TaskScope, ValidIssuesBySeverity,
    };

    use crate::command::{Cli, Commands};

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        id: &str,
        implementer: (&str, &str),
        reviewer: (&str, &str),
        arbiter: (&str, &str),
        impl_score: f32,
        rev_score: f32,
        merged: bool,
        fix_iters: u32,
        scope: TaskScope,
        ambiguity: Option<Ambiguity>,
    ) -> DuelRun {
        DuelRun {
            run_id: id.to_string(),
            task_id: format!("T-{id}"),
            completed_at: ts("2026-04-09T04:00:00Z"),
            task_class: TaskClass {
                scope,
                ambiguity,
                source: "derived".to_string(),
            },
            roles: Roles {
                implementer: RoleAssignment {
                    agent: implementer.0.to_string(),
                    model: implementer.1.to_string(),
                },
                reviewer: RoleAssignment {
                    agent: reviewer.0.to_string(),
                    model: reviewer.1.to_string(),
                },
                arbiter: RoleAssignment {
                    agent: arbiter.0.to_string(),
                    model: arbiter.1.to_string(),
                },
            },
            outcome: Outcome {
                decision: if merged {
                    Decision::Approved
                } else {
                    Decision::RequestChanges
                },
                fix_loop_iterations: fix_iters,
                fix_loop_exhausted: fix_iters >= 3,
                pr_number: Some(100),
                merged,
            },
            scores: Scores {
                implementer_score: impl_score,
                reviewer_score: rev_score,
            },
            reviewer_stats: ReviewerStats {
                total_comments: 0,
                valid: 0,
                invalid: 0,
                out_of_scope: 0,
                nitpick: 0,
                precision: 0.0,
                arbiter_override_rate: 0.0,
            },
            implementer_stats: ImplementerStats {
                valid_issues_against: ValidIssuesBySeverity::default(),
            },
            cost: Cost {
                wall_clock_seconds: 600,
                tokens_in: None,
                tokens_out: None,
            },
        }
    }

    fn seeded_runs() -> Vec<DuelRun> {
        vec![
            run(
                "r1",
                ("claude", "opus"),
                ("codex", "gpt-5.4"),
                ("gemini", "gemini-3.1-pro"),
                5.0,
                4.0,
                true,
                0,
                TaskScope::SingleFile,
                Some(Ambiguity::WellSpecified),
            ),
            run(
                "r2",
                ("claude", "opus"),
                ("gemini", "gemini-3.1-pro"),
                ("codex", "gpt-5.4"),
                3.0,
                5.0,
                true,
                1,
                TaskScope::MultiFile,
                Some(Ambiguity::NeedsJudgment),
            ),
            run(
                "r3",
                ("codex", "gpt-5.4"),
                ("claude", "opus"),
                ("gemini", "gemini-3.1-pro"),
                2.0,
                2.5,
                false,
                3,
                TaskScope::CrossCrate,
                None,
            ),
        ]
    }

    #[test]
    fn duel_list_accepts_full_flag() {
        let cli =
            Cli::try_parse_from(["orbit", "duel", "list", "--full"]).expect("duel list parses");

        let Commands::Duel(duel) = cli.command else {
            panic!("expected duel command");
        };
        let DuelSubcommand::List(args) = duel.command else {
            panic!("expected duel list command");
        };
        assert!(args.full);
    }

    #[test]
    fn aggregate_none_segment_all_roles_produces_three_rows_per_run() {
        let runs = seeded_runs();
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::None,
                role: None,
            },
        );
        // 3 runs × 3 roles = 9 role-rows; aggregate folds by
        // (segment, role, agent, model). Exact row count depends on how
        // many (role, agent, model) keys collided.
        assert!(!aggs.rows.is_empty());
        let total_runs: u64 = aggs.rows.iter().map(|r| r.runs as u64).sum();
        assert_eq!(
            total_runs, 9,
            "every run contributes to all three role rows"
        );
    }

    #[test]
    fn filter_by_implementer_role_collapses_rows_to_implementers_only() {
        let runs = seeded_runs();
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::None,
                role: Some(RoleAxis::Implementer),
            },
        );
        for row in &aggs.rows {
            assert_eq!(row.role, "implementer");
        }
        let total_runs: u64 = aggs.rows.iter().map(|r| r.runs as u64).sum();
        assert_eq!(total_runs, 3);
    }

    #[test]
    fn segment_by_scope_produces_distinct_segments_per_scope_value() {
        let runs = seeded_runs();
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::Scope,
                role: Some(RoleAxis::Implementer),
            },
        );
        let segments: std::collections::BTreeSet<&str> =
            aggs.rows.iter().map(|r| r.segment.as_str()).collect();
        assert!(segments.contains("single_file"));
        assert!(segments.contains("multi_file"));
        assert!(segments.contains("cross_crate"));
    }

    #[test]
    fn segment_by_ambiguity_buckets_null_as_unknown() {
        let runs = seeded_runs();
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::Ambiguity,
                role: Some(RoleAxis::Reviewer),
            },
        );
        let segments: std::collections::BTreeSet<&str> =
            aggs.rows.iter().map(|r| r.segment.as_str()).collect();
        assert!(segments.contains("well_specified"));
        assert!(segments.contains("needs_judgment"));
        assert!(segments.contains("unknown"));
    }

    #[test]
    fn empty_runs_yields_empty_aggregates_not_an_error() {
        let runs: Vec<DuelRun> = Vec::new();
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::None,
                role: None,
            },
        );
        assert!(aggs.rows.is_empty());
    }

    #[test]
    fn implementer_merge_rate_matches_manual_calculation() {
        let runs = seeded_runs();
        // claude-opus implementer: r1 merged=true, r2 merged=true → 2/2 = 1.0
        // codex-gpt-5.4 implementer: r3 merged=false → 0/1 = 0.0
        let aggs = aggregate(
            &runs,
            AggregateFilter {
                segment_by: SegmentBy::None,
                role: Some(RoleAxis::Implementer),
            },
        );
        let claude_row = aggs
            .rows
            .iter()
            .find(|r| r.agent == "claude" && r.model == "opus")
            .expect("claude implementer row");
        assert_eq!(claude_row.runs, 2);
        assert!((claude_row.merge_rate - 1.0).abs() < 1e-9);
        assert!((claude_row.avg_score - 4.0).abs() < 1e-9); // (5 + 3) / 2

        let codex_row = aggs
            .rows
            .iter()
            .find(|r| r.agent == "codex" && r.model == "gpt-5.4")
            .expect("codex implementer row");
        assert_eq!(codex_row.runs, 1);
        assert!((codex_row.merge_rate - 0.0).abs() < 1e-9);
    }

    #[test]
    fn duel_score_parser_preserves_existing_flags() {
        let cli = Cli::try_parse_from([
            "orbit", "duel", "score", "--by", "scope", "--role", "reviewer", "--json",
        ])
        .expect("duel score should parse");

        let Commands::Duel(duel) = cli.command else {
            panic!("expected duel command");
        };
        let DuelSubcommand::Score(args) = duel.command else {
            panic!("expected duel score subcommand");
        };
        assert!(matches!(args.by, SegmentByArg::Scope));
        assert!(matches!(args.role, RoleFilterArg::Reviewer));
        assert!(args.json);
    }

    #[test]
    fn duel_run_parser_accepts_optional_task_id() {
        let cli = Cli::try_parse_from(["orbit", "duel", "run", "T20260409-0310"])
            .expect("duel run should parse");

        let Commands::Duel(duel) = cli.command else {
            panic!("expected duel command");
        };
        let DuelSubcommand::Run(args) = duel.command else {
            panic!("expected duel run subcommand");
        };
        assert_eq!(args.task_id.as_deref(), Some("T20260409-0310"));
    }

    #[test]
    fn duel_show_accepts_missing_run_id() {
        let cli = Cli::try_parse_from(["orbit", "duel", "show"]).expect("duel show should parse");

        let Commands::Duel(duel) = cli.command else {
            panic!("expected duel command");
        };
        let DuelSubcommand::Show(args) = duel.command else {
            panic!("expected duel show subcommand");
        };
        assert!(args.run_id.is_none());
    }
}

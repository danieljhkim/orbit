use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use orbit_core::knowledge_stats::{KnowledgeStatsSummary, aggregate as aggregate_knowledge_stats};
use orbit_core::{
    ActivityInvocationMetrics, InvocationQuery, InvocationRecord, OrbitError, OrbitRuntime,
    TaskInvocationMetrics, ToolInvocationMetrics,
};
use serde_json::json;

use crate::command::Execute;

const LIMITATIONS_HELP: &str = "Known limitations:\n  - Subagent attribution folds into the parent invocation totals.\n  - cache_read_tokens are reported separately from input_tokens.\n  - Multi-task invocations are fully attributed to every tagged task.\n  - Trace completeness depends on provider CLI output shape; unsupported providers may still persist zero traces.";

#[derive(Args)]
#[command(
    about = "Inspect token, tool-call, and knowledge-pack metrics",
    after_help = LIMITATIONS_HELP
)]
pub struct MetricsCommand {
    /// Limit the number of most-recent job runs considered for knowledge metrics in the overview.
    #[arg(long)]
    pub limit: Option<usize>,
    /// Output overview as JSON
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<MetricsSubcommand>,
}

impl Execute for MetricsCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            Some(command) => command.execute(runtime),
            None => MetricsOverviewArgs {
                limit: self.limit,
                json: self.json,
            }
            .execute(runtime),
        }
    }
}

#[derive(Subcommand)]
pub enum MetricsSubcommand {
    /// Show an overview of knowledge, activity, and tool metrics
    Overview(MetricsOverviewArgs),
    /// Show knowledge-pack usage and savings metrics
    Knowledge(MetricsKnowledgeArgs),
    /// Show token rollups grouped by activity
    Activity(MetricsActivityArgs),
    /// Show token rollups for a task
    Task(MetricsTaskArgs),
    /// Show tool call frequency and result sizes grouped by activity
    Tools(MetricsToolsArgs),
    /// Show raw invocation records with filters
    Invocations(MetricsInvocationsArgs),
}

impl Execute for MetricsSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            MetricsSubcommand::Overview(args) => args.execute(runtime),
            MetricsSubcommand::Knowledge(args) => args.execute(runtime),
            MetricsSubcommand::Activity(args) => args.execute(runtime),
            MetricsSubcommand::Task(args) => args.execute(runtime),
            MetricsSubcommand::Tools(args) => args.execute(runtime),
            MetricsSubcommand::Invocations(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct MetricsOverviewArgs {
    /// Limit the number of most-recent job runs considered for knowledge metrics
    #[arg(long)]
    pub limit: Option<usize>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsOverviewArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let knowledge = load_knowledge_summary(runtime, self.limit)?;
        let activity = runtime.activity_invocation_metrics()?;
        let tools = runtime.tool_invocation_metrics()?;

        if self.json {
            return crate::output::json::print_pretty(&json!({
                "knowledge": knowledge,
                "activity": activity,
                "tools": tools,
            }));
        }

        print_knowledge_summary(&knowledge);
        println!();
        print_activity_rows(&activity);
        println!();
        print_tool_rows(&tools);
        Ok(())
    }
}

#[derive(Args)]
pub struct MetricsKnowledgeArgs {
    /// Limit the number of most-recent job runs considered
    #[arg(long)]
    pub limit: Option<usize>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsKnowledgeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let summary = load_knowledge_summary(runtime, self.limit)?;

        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&summary).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }

        print_knowledge_summary(&summary);
        Ok(())
    }
}

#[derive(Args)]
pub struct MetricsActivityArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsActivityArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let rows = runtime.activity_invocation_metrics()?;
        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&rows).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }
        print_activity_rows(&rows);
        Ok(())
    }
}

#[derive(Args)]
pub struct MetricsTaskArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsTaskArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let row = runtime.task_invocation_metrics(&self.id)?;
        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&row).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }
        print_task_metrics(&row);
        Ok(())
    }
}

#[derive(Args)]
pub struct MetricsToolsArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsToolsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let rows = runtime.tool_invocation_metrics()?;
        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&rows).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }
        print_tool_rows(&rows);
        Ok(())
    }
}

#[derive(Args, Default)]
pub struct MetricsInvocationsArgs {
    /// Include invocations on or after this RFC3339 timestamp
    #[arg(long)]
    pub since: Option<String>,
    /// Include invocations on or before this RFC3339 timestamp
    #[arg(long)]
    pub until: Option<String>,
    /// Filter by job run ID
    #[arg(long)]
    pub job_run_id: Option<String>,
    /// Filter by activity ID
    #[arg(long)]
    pub activity_id: Option<String>,
    /// Filter by task ID
    #[arg(long)]
    pub task_id: Option<String>,
    /// Filter by agent family
    #[arg(long)]
    pub agent: Option<String>,
    /// Filter by model
    #[arg(long)]
    pub model: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool_name: Option<String>,
    /// Limit the number of invocations returned
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for MetricsInvocationsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let query = InvocationQuery {
            since: parse_rfc3339_opt(self.since, "since")?,
            until: parse_rfc3339_opt(self.until, "until")?,
            job_run_id: self.job_run_id,
            activity_id: self.activity_id,
            task_id: self.task_id,
            agent: self.agent,
            model: self.model,
            tool_name: self.tool_name,
            limit: self.limit,
        };
        let rows = runtime.invocation_records(query)?;
        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&rows).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }
        print_invocation_rows(&rows);
        Ok(())
    }
}

fn load_knowledge_summary(
    runtime: &OrbitRuntime,
    limit: Option<usize>,
) -> Result<KnowledgeStatsSummary, OrbitError> {
    let runs = runtime.list_job_runs(orbit_core::command::job_run::JobRunListParams {
        limit,
        ..Default::default()
    })?;
    Ok(aggregate_knowledge_stats(&runs))
}

fn print_knowledge_summary(summary: &KnowledgeStatsSummary) {
    if summary.total_runs == 0 {
        println!("No knowledge usage metrics found.");
        return;
    }

    println!("orbit metrics knowledge");
    println!();
    println!(
        "Runs:        {} total | {} with pack | {} fallback ({:.1}% fallback rate)",
        summary.total_runs,
        summary.pack_runs,
        summary.fallback_runs,
        summary.fallback_rate * 100.0
    );
    println!();

    if let Some(compression) = &summary.compression {
        println!("Compression (tokenized, cl100k_base):");
        println!("  Mean:      {:.1}x", compression.mean);
        println!("  p50:       {:.1}x", compression.p50);
        println!("  p90:       {:.1}x", compression.p90);
        println!("  Min:       {:.1}x", compression.min);
        println!();
    }

    println!("Double-read guard:");
    println!(
        "  Mean rate: {:.2} ({:.0}% of baseline tokens re-read via fs.read)",
        summary.double_read.mean_rate,
        summary.double_read.mean_rate * 100.0
    );
    println!(
        "  Runs >50%: {} / {} (flag for investigation)",
        summary.double_read.runs_over_fifty_percent, summary.double_read.measured_runs
    );
    println!();

    println!("Total LLM input tokens per activity:");
    println!(
        "  With pack:    avg {:.0} tokens",
        summary.total_llm_input_tokens.with_pack_avg
    );
    println!(
        "  Without pack: avg {:.0} tokens",
        summary.total_llm_input_tokens.without_pack_avg
    );
    if let Some(savings) = summary.total_llm_input_tokens.estimated_savings {
        println!("  Estimated savings: {:.0}%", savings * 100.0);
    }
}

fn print_activity_rows(rows: &[ActivityInvocationMetrics]) {
    if rows.is_empty() {
        println!("No invocation metrics found.");
        return;
    }

    println!("Activity Metrics:");
    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "ACTIVITY",
        "AGENT",
        "MODEL",
        "INVOCATIONS",
        "AVG",
        "P50",
        "P95",
        "TOTAL",
        "INPUT",
        "CACHE READ",
        "CACHE CREATE",
        "OUTPUT",
        "TOOLS",
    ]);
    for row in rows {
        table.add_row(vec![
            Cell::new(&row.activity_id),
            Cell::new(&row.agent),
            Cell::new(row.model.as_deref().unwrap_or("-")),
            Cell::new(row.invocation_count),
            Cell::new(format_decimal(row.avg_tokens)),
            Cell::new(row.p50_tokens),
            Cell::new(row.p95_tokens),
            Cell::new(row.total_tokens),
            Cell::new(row.total_input_tokens),
            Cell::new(row.total_cache_read_tokens),
            Cell::new(row.total_cache_create_tokens),
            Cell::new(row.total_output_tokens),
            Cell::new(row.total_tool_calls),
        ]);
    }
    println!("{table}");
}

fn print_task_metrics(row: &TaskInvocationMetrics) {
    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "TASK",
        "INVOCATIONS",
        "TOTAL",
        "INPUT",
        "CACHE READ",
        "CACHE CREATE",
        "OUTPUT",
        "TOOLS",
    ]);
    table.add_row(vec![
        Cell::new(&row.task_id),
        Cell::new(row.invocation_count),
        Cell::new(row.total_tokens),
        Cell::new(row.total_input_tokens),
        Cell::new(row.total_cache_read_tokens),
        Cell::new(row.total_cache_create_tokens),
        Cell::new(row.total_output_tokens),
        Cell::new(row.total_tool_calls),
    ]);
    println!("{table}");
}

fn print_tool_rows(rows: &[ToolInvocationMetrics]) {
    if rows.is_empty() {
        println!("No tool call metrics found.");
        return;
    }

    println!("Tool Metrics:");
    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "ACTIVITY",
        "TOOL",
        "CALLS",
        "AVG RESULT BYTES",
        "TOTAL RESULT BYTES",
    ]);
    for row in rows {
        table.add_row(vec![
            Cell::new(&row.activity_id),
            Cell::new(&row.tool_name),
            Cell::new(row.call_count),
            Cell::new(format_decimal(row.avg_result_bytes)),
            Cell::new(row.total_result_bytes),
        ]);
    }
    println!("{table}");
}

fn print_invocation_rows(rows: &[InvocationRecord]) {
    if rows.is_empty() {
        println!("No invocation records found.");
        return;
    }

    println!("Invocation Records:");
    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "TS",
        "JOB RUN",
        "ACTIVITY",
        "AGENT",
        "MODEL",
        "TOTAL",
        "INPUT",
        "CACHE READ",
        "CACHE CREATE",
        "OUTPUT",
        "TOOLS",
        "TASKS",
    ]);
    for row in rows {
        table.add_row(vec![
            Cell::new(format_table_timestamp(&row.ts)),
            Cell::new(&row.job_run_id),
            Cell::new(&row.activity_id),
            Cell::new(&row.agent),
            Cell::new(row.model.as_deref().unwrap_or("-")),
            Cell::new(row.total_tokens),
            Cell::new(row.input_tokens),
            Cell::new(row.cache_read_tokens),
            Cell::new(row.cache_create_tokens),
            Cell::new(row.output_tokens),
            Cell::new(row.tool_call_count),
            Cell::new(row.task_ids.len()),
        ]);
    }
    println!("{table}");
}

fn format_decimal(value: f64) -> String {
    format!("{value:.1}")
}

fn format_table_timestamp(value: &DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn parse_rfc3339_opt(
    value: Option<String>,
    field_name: &str,
) -> Result<Option<DateTime<Utc>>, OrbitError> {
    match value {
        Some(raw) => DateTime::parse_from_rfc3339(&raw)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|error| OrbitError::InvalidInput(format!("invalid {field_name}: {error}"))),
        None => Ok(None),
    }
}

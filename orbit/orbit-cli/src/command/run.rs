use clap::Args;
use orbit_core::{
    OrbitError, OrbitRuntime, WORKFLOWS, WorkflowInput, build_workflow_input, find_workflow,
    validate_workflow_flags,
};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
#[command(
    about = "Run a first-class workflow",
    after_help = "Examples:\n  orbit run ship\n  orbit run ship --tasks T123,T456 --parallelism 2\n  orbit run ship-local --base main\n  orbit run review\n  orbit run review-pr --pr-number 42 --base main\n  orbit run --list"
)]
pub struct RunCommand {
    /// Workflow name (e.g. ship, ship-local, review, review-pr)
    pub workflow: Option<String>,

    /// Comma-separated task IDs to process (omit to auto-select from backlog)
    #[arg(long)]
    pub tasks: Option<String>,

    /// Number of parallel workers
    #[arg(long)]
    pub parallelism: Option<u32>,

    /// Base branch for the pipeline
    #[arg(long)]
    pub base: Option<String>,

    /// Pull request number for review-pr workflows
    #[arg(long = "pr-number")]
    pub pr_number: Option<String>,

    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// List available workflows
    #[arg(long)]
    pub list: bool,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.list || self.workflow.is_none() {
            return print_workflow_list();
        }

        let name = self.workflow.as_deref().unwrap();
        let workflow = find_workflow(name).ok_or_else(|| {
            let available = WORKFLOWS
                .iter()
                .map(|w| w.alias)
                .collect::<Vec<_>>()
                .join(", ");
            OrbitError::InvalidInput(format!("unknown workflow '{name}'. Available: {available}"))
        })?;

        let wf_input = WorkflowInput {
            tasks: self.tasks.clone(),
            parallelism: self.parallelism,
            base: self.base.clone(),
            pr_number: self.pr_number.clone(),
        };
        validate_workflow_flags(workflow, &wf_input)?;
        let input = build_workflow_input(&wf_input)?;

        let run = runtime.run_job_now_with_input_debug(workflow.job_id, input, self.debug)?;
        let run_details = runtime
            .job_history(workflow.job_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);

        if self.json {
            crate::output::json::print_pretty(&json!({
                "workflow": workflow.alias,
                "job_id": run.job_id,
                "run_id": run.run_id,
                "state": run.state.to_string(),
                "attempt": run.attempt,
                "error_code": run_details.as_ref().and_then(|e| e.steps.last()).and_then(|s| s.error_code.clone()),
                "error_message": run_details.as_ref().and_then(|e| e.steps.last()).and_then(|s| s.error_message.clone()),
            }))
        } else {
            let error_code = run_details
                .as_ref()
                .and_then(|e| e.steps.last())
                .and_then(|s| s.error_code.clone())
                .unwrap_or_else(|| "-".to_string());
            let error_message = run_details
                .as_ref()
                .and_then(|e| e.steps.last())
                .and_then(|s| s.error_message.clone())
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
}

fn print_workflow_list() -> Result<(), OrbitError> {
    let mut table = crate::output::table::build_table(&["WORKFLOW", "JOB", "DESCRIPTION"]);
    for w in WORKFLOWS {
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(w.alias),
            Cell::new(w.job_id),
            Cell::new(w.description),
        ]);
    }
    println!("{table}");
    Ok(())
}

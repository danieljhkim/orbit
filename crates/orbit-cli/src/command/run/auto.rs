//! `orbit run auto` workspace logistics entrypoint.

use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute};
use crate::parse::parse_duration_seconds;

use super::support::{WorkflowDispatchResult, print_workflow_dispatch_results};

pub(super) const AUTO_WORKFLOW: &str = "auto";

#[derive(Args)]
#[command(
    about = "Drain the workspace backlog for a window (loose leaves, plus one epic)",
    override_usage = "orbit run auto [OPTIONS]",
    after_help = "Examples:\n  orbit run auto\n  orbit run auto --for 4h\n  orbit run auto --for 4h --concurrency 8\n\n\
                  The drain re-lists the whole backlog every pass and keeps `--concurrency`\n\
                  tasks in flight, starting a replacement as each one finishes rather than\n\
                  waiting for the batch. An epic root runs alongside the leaves, one at a time.\n\n\
                  Inspect submitted runs with `orbit run history -j workspace_auto_pipeline` and\n\
                  `orbit run show <RUN_ID>`."
)]
pub struct AutoCommand {
    /// How long to keep draining, e.g. `30m`, `2h`. Without it the run takes
    /// one tick and stops. The window bounds only the start of new work: a
    /// task already being shipped when it expires still finishes.
    #[arg(long = "for", value_name = "DURATION")]
    pub for_duration: Option<String>,
    /// How many tasks may be in flight at once. The drain tops these slots up
    /// from the whole backlog as each one frees, so this is the parallelism,
    /// not a batch size. Defaults to 5.
    #[arg(long, value_name = "N")]
    pub concurrency: Option<u32>,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
    /// Token for this workspace's exclusive claim, when another operator holds
    /// one. Falls back to `ORBIT_WORKSPACE_CLAIM_TOKEN`.
    #[arg(long)]
    pub claim_token: Option<String>,
}

impl Execute for AutoCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let for_seconds = self
            .for_duration
            .as_deref()
            .map(parse_duration_seconds)
            .transpose()?;
        let invoke = runtime.submit_workspace_auto_run(
            for_seconds,
            self.concurrency,
            None,
            self.claim_token.as_deref(),
        )?;
        let run = WorkflowDispatchResult {
            workflow_alias: AUTO_WORKFLOW,
            job_id: invoke.job_name,
            run_id: invoke.run_id,
            state: if invoke.queued {
                "queued".to_string()
            } else {
                "submitted".to_string()
            },
            attempt: 1,
            error_code: None,
            error_message: None,
        };
        print_workflow_dispatch_results(AUTO_WORKFLOW, &[run], self.json)?;
        Ok(CommandOutput::Silent)
    }
}

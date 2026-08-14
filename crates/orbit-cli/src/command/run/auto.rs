//! `orbit run auto` workspace logistics entrypoint.

use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute};

use super::support::{WorkflowDispatchResult, print_workflow_dispatch_results};

pub(super) const AUTO_WORKFLOW: &str = "auto";

#[derive(Args)]
#[command(
    about = "Run one workspace logistics tick (loose leaves, then one epic)",
    override_usage = "orbit run auto [OPTIONS]",
    after_help = "Inspect submitted runs with `orbit run history -j workspace_auto_pipeline` and `orbit run show <RUN_ID>`."
)]
pub struct AutoCommand {
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
        let invoke = runtime.submit_workspace_auto_run(None, self.claim_token.as_deref())?;
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

use clap::Args;
use orbit_core::runtime::run_audit::RunCliInvocationRecord;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::{CommandOut, CommandOutput, Execute};

use super::steps::{resolve_run, resolve_step_filter};

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run_id\":\"...\",\"job_id\":\"...\",\"records\":[{\"step_id\":...,\"stdout_blob_ref\":...,\"stderr_blob_ref\":...,\"stdout\":\"...\",\"stderr\":\"...\"}]}\nExamples:\n  orbit run logs\n  orbit run logs jrun-20260426-0631\n  orbit run logs jrun-20260426-0631 -s implement_one --json"
)]
pub struct RunLogsArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Show raw logs for a single activity step.id from the v2 job YAML
    #[arg(short = 's', long = "step")]
    pub step_id: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunLogsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        {
            print_run_logs(
                runtime,
                self.run_id.as_deref(),
                self.step_id.as_deref(),
                self.json,
            )?;
            Ok(CommandOutput::Silent)
        }
    }
}

fn print_run_logs(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let audit_steps = runtime.collect_run_audit_steps(&run.run_id)?;
    let step_filter = resolve_step_filter(&run, &audit_steps, step_id)?;
    let records = filter_cli_invocation_records(
        runtime.collect_run_cli_invocations(&run.run_id)?,
        step_filter.as_deref(),
    );

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run_id": run.run_id,
            "job_id": run.job_id,
            "records": records.iter().map(cli_invocation_record_to_json).collect::<Vec<_>>(),
        }));
    }

    if records.is_empty() {
        println!("No raw stdout/stderr blobs recorded.");
        return Ok(());
    }

    for record in &records {
        print!("{}", record.stdout);
        eprint!("{}", record.stderr);
    }
    Ok(())
}

fn filter_cli_invocation_records(
    records: Vec<RunCliInvocationRecord>,
    step_filter: Option<&str>,
) -> Vec<RunCliInvocationRecord> {
    records
        .into_iter()
        .filter(|record| step_filter.is_none_or(|filter| record.step_id.as_deref() == Some(filter)))
        .collect()
}

fn cli_invocation_record_to_json(record: &RunCliInvocationRecord) -> Value {
    json!({
        "step_id": record.step_id,
        "provider": record.provider,
        "stdout_blob_ref": record.stdout_blob_ref,
        "stderr_blob_ref": record.stderr_blob_ref,
        "stdout": record.stdout,
        "stderr": record.stderr,
    })
}

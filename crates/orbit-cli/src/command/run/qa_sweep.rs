//! `orbit run qa-sweep` — trailing QA validation over direct-push workspaces
//! [ORB-10039], sibling of `orbit run ship-sweep`. Designed for unattended
//! schedulers: it never bootstraps a workspace from the caller's cwd,
//! isolates per-workspace failures, and exits non-zero only when a workspace
//! errored (a red check is the sweep working — it files a task — not a sweep
//! failure).

use clap::Args;
use orbit_core::OrbitError;
use orbit_core::qa::{QaCheckReport, QaSweepOptions, QaWorkspaceReport, run_qa_sweep};
use serde_json::{Value, json};

#[derive(Args)]
#[command(
    name = "qa-sweep",
    about = "Validate new agent-main commits in configured direct-push workspaces",
    after_help = "Workspaces and their checks come from the [qa] section of the GLOBAL\n\
                  ~/.orbit/config.toml (never workspace config, which task-mutation\n\
                  commands rewrite). Per workspace: diff the live checkout's HEAD against\n\
                  the last-validated watermark; when new commits exist, run the checks,\n\
                  file fingerprint-deduped orbit tasks for failures, and advance the\n\
                  watermark only on a fully green pass. Intended to run from a scheduler\n\
                  (systemd timer / cron), e.g.:\n  orbit run qa-sweep --json\n\n\
                  Sweeps are recorded in each workspace's run ledger under job id\n\
                  'qa_sweep': inspect with `orbit run history -j qa_sweep` and\n\
                  `orbit run show <run_id>` from the workspace."
)]
pub struct QaSweepCommand {
    /// Report what would be validated without running checks, recording runs,
    /// filing tasks, or advancing watermarks.
    #[arg(long)]
    pub dry_run: bool,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl QaSweepCommand {
    /// Runs without a pre-initialized runtime: the sweep resolves every
    /// workspace from the global registry and must never bootstrap a
    /// `.orbit/` in the scheduler's working directory.
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let outcome = run_qa_sweep(QaSweepOptions {
            dry_run: self.dry_run,
            workspace: None,
        })?;

        let failed = outcome
            .reports
            .iter()
            .filter(|report| report.action == "error")
            .count();

        if self.json {
            crate::output::json::print_pretty(&json!({
                "dry_run": self.dry_run,
                "lock_busy": outcome.lock_busy,
                "workspaces": outcome.reports.len(),
                "validated": count_actions(&outcome.reports, "validated"),
                "failed_checks": count_actions(&outcome.reports, "failed"),
                "would_validate": count_actions(&outcome.reports, "would_validate"),
                "skipped": count_actions(&outcome.reports, "skipped"),
                "errors": failed,
                "reports": outcome.reports.iter().map(report_json).collect::<Vec<_>>(),
            }))?;
        } else if outcome.lock_busy {
            println!("qa-sweep: another pass holds the lock on this host; exiting");
        } else if outcome.reports.is_empty() {
            println!("qa-sweep: no [qa] workspaces configured in the global config.toml");
        } else {
            for report in &outcome.reports {
                println!("{}", report_line(report));
            }
        }

        if failed > 0 {
            return Err(OrbitError::WorkspaceError(format!(
                "qa-sweep: {failed} of {} workspace(s) errored",
                outcome.reports.len()
            )));
        }
        Ok(())
    }
}

fn count_actions(reports: &[QaWorkspaceReport], action: &str) -> usize {
    reports.iter().filter(|r| r.action == action).count()
}

pub(crate) fn report_json(report: &QaWorkspaceReport) -> Value {
    json!({
        "workspace": report.workspace,
        "action": report.action,
        "reason": report.reason,
        "branch": report.branch,
        "head": report.head,
        "baseline": report.baseline,
        "new_commits": report.new_commits.as_ref().map(Vec::len),
        "watermark_reset": report.watermark_reset,
        "run_id": report.run_id,
        "checks": report.checks.iter().map(|check| json!({
            "name": check.name,
            "outcome": check.outcome,
            "exit_code": check.exit_code,
            "duration_ms": check.duration_ms,
            "fingerprint": check.fingerprint,
            "filed_task": check.filed_task,
            "deduped_task": check.deduped_task,
        })).collect::<Vec<_>>(),
    })
}

pub(crate) fn report_line(report: &QaWorkspaceReport) -> String {
    let mut line = format!("{}: {}", report.workspace, report.action);
    if let Some(reason) = &report.reason {
        line.push_str(&format!(" — {reason}"));
    }
    if let (Some(baseline), Some(head)) = (&report.baseline, &report.head)
        && report.action != "skipped"
    {
        line.push_str(&format!(" — {}..{}", short_sha(baseline), short_sha(head)));
    } else if let Some(head) = &report.head
        && report.action != "skipped"
    {
        line.push_str(&format!(" — HEAD {}", short_sha(head)));
    }
    if !report.checks.is_empty() {
        let checks = report
            .checks
            .iter()
            .map(check_summary)
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(" [{checks}]"));
    }
    if let Some(run_id) = &report.run_id {
        line.push_str(&format!(" — run {run_id}"));
    }
    line
}

fn check_summary(check: &QaCheckReport) -> String {
    let mut summary = format!("{}: {}", check.name, check.outcome);
    if let Some(task) = &check.filed_task {
        summary.push_str(&format!(" (filed {task})"));
    }
    if let Some(task) = &check.deduped_task {
        summary.push_str(&format!(" (open {task})"));
    }
    summary
}

fn short_sha(sha: &str) -> &str {
    if sha.len() > 10 { &sha[..10] } else { sha }
}

//! `orbit run qa-sweep` — the trailing QA pass over direct-push workspaces
//! [ORB-10039], sibling of `orbit run ship-sweep` (design D4).
//!
//! Direct pushes to `agent-main` stay fast at write time; this sweep enforces
//! correctness on a lag. Per configured workspace it diffs the live checkout's
//! HEAD against a per-workspace last-validated watermark, runs the configured
//! checks when new commits exist, files fingerprint-deduped orbit tasks for
//! failures, and advances the watermark only on a fully green pass.
//!
//! **Ledger integration.** Every validating pass records a first-class v2 job
//! run (job id [`QA_SWEEP_JOB`]) in the swept workspace's jobs store — one run
//! per workspace per pass, one run step per executed check — via the same
//! store the pipeline worker uses (`insert_run` → `mark_run_running` → per-
//! check `complete_run_step` → `finalize_run`, plus the `JobRunStarted` /
//! `JobRunCompleted` events). Checks execute inline in the sweep process
//! (they are host-trusted shell commands, not agent activities), so no
//! worker is spawned and no v2 job YAML asset exists; `orbit run history`
//! intentionally serves run history for asset-less job ids, so
//! `orbit run history -j qa_sweep` and `orbit run show <run_id>` surface the
//! sweeps and their per-check steps honestly — the run record is written by
//! the code that did the work, not a decorative side channel.
//!
//! Like ship-sweep, this never bootstraps a `.orbit/` in the scheduler's cwd:
//! everything resolves from the global registry and global config, and
//! per-workspace failures are isolated into report rows.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{
    JobRunState, JobTargetType, OrbitError, OrbitEvent, Task, TaskStatus, TaskType, Workspace,
    WorkspaceStatus,
};
use orbit_store::JobRunStepParams;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;
use crate::config::RuntimeConfig;
use crate::workspace_registry;

use super::config::{QaCheck, QaSweepConfig, QaWorkspaceConfig};
use super::fingerprint::{QA_SWEEP_TAG, failure_fingerprint, fingerprint_tag};
use super::git;
use super::state::{QaWorkspaceWatermark, advance_watermark, load_state, state_path};

/// Job id qa-sweep runs are recorded under in each workspace's run ledger.
pub const QA_SWEEP_JOB: &str = "qa_sweep";

/// Commits listed as evidence per finding (range summaries are capped; the
/// range endpoints are always recorded in full).
const EVIDENCE_COMMIT_LIMIT: usize = 20;
/// Head of the combined check output quoted as evidence in filed tasks.
const EVIDENCE_OUTPUT_LINES: usize = 40;
const EVIDENCE_OUTPUT_BYTES: usize = 4000;
/// Poll interval while waiting on a running check.
const CHECK_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Options for one qa-sweep pass.
#[derive(Debug, Clone, Default)]
pub struct QaSweepOptions {
    /// Report what would be validated without running checks, recording runs,
    /// filing tasks, or advancing watermarks.
    pub dry_run: bool,
    /// Restrict the pass to one configured workspace. `None` preserves the
    /// host-wide behavior used by the transitional systemd entry point.
    pub workspace: Option<String>,
}

/// Result of one qa-sweep pass.
#[derive(Debug, Default)]
pub struct QaSweepOutcome {
    /// True when another pass held the host lock and this one exited early.
    pub lock_busy: bool,
    /// Per-configured-workspace outcomes, in config order.
    pub reports: Vec<QaWorkspaceReport>,
}

/// Per-workspace outcome of one pass.
#[derive(Debug)]
pub struct QaWorkspaceReport {
    /// Configured workspace name.
    pub workspace: String,
    /// One of: `validated`, `failed`, `would_validate`, `skipped`, `error`.
    pub action: &'static str,
    /// Why, for `skipped` / `error` rows.
    pub reason: Option<String>,
    /// Branch the checkout was validated on.
    pub branch: Option<String>,
    /// HEAD sha the checks ran against.
    pub head: Option<String>,
    /// Watermark sha the diff was computed from (`None` = first validation).
    pub baseline: Option<String>,
    /// Commits in `baseline..head`, capped at [`EVIDENCE_COMMIT_LIMIT`].
    /// `None` when the baseline no longer resolves (history rewrite).
    pub new_commits: Option<Vec<String>>,
    /// True when the watermark existed but no longer resolves in the repo.
    pub watermark_reset: bool,
    /// Ledger run id, when a run was recorded.
    pub run_id: Option<String>,
    /// Per-check outcomes for validating passes.
    pub checks: Vec<QaCheckReport>,
}

impl QaWorkspaceReport {
    fn skipped(workspace: &str, reason: &str) -> Self {
        Self {
            reason: Some(reason.to_string()),
            ..Self::bare(workspace, "skipped")
        }
    }

    fn bare(workspace: &str, action: &'static str) -> Self {
        Self {
            workspace: workspace.to_string(),
            action,
            reason: None,
            branch: None,
            head: None,
            baseline: None,
            new_commits: None,
            watermark_reset: false,
            run_id: None,
            checks: Vec::new(),
        }
    }
}

/// One check's outcome within a validating pass.
#[derive(Debug)]
pub struct QaCheckReport {
    /// Configured check name.
    pub name: String,
    /// One of: `passed`, `failed`, `timeout`, `muted`, `would_run`.
    pub outcome: &'static str,
    /// Exit code, when the check ran to completion.
    pub exit_code: Option<i32>,
    /// Wall-clock duration of the check.
    pub duration_ms: u64,
    /// Failure fingerprint, for failing checks.
    pub fingerprint: Option<String>,
    /// Task id filed for this failure, when one was created this pass.
    pub filed_task: Option<String>,
    /// Open task id the failure deduped against, when one already existed.
    pub deduped_task: Option<String>,
}

/// Run one qa-sweep pass against the default global root (`~/.orbit`).
pub fn run_qa_sweep(options: QaSweepOptions) -> Result<QaSweepOutcome, OrbitError> {
    let global_root = workspace_registry::global_orbit_dir()?;
    run_qa_sweep_at(&global_root, options)
}

/// Run one qa-sweep pass against an explicit global root (test seam).
pub fn run_qa_sweep_at(
    global_root: &Path,
    options: QaSweepOptions,
) -> Result<QaSweepOutcome, OrbitError> {
    // Host-level config only: workspace config.toml files are rewritten by
    // task-mutation commands and must never own scheduler enablement.
    let config = RuntimeConfig::load_layered(global_root, global_root)?;
    let qa = config.qa_sweep().clone();

    // One pass per host at a time; flock releases on process death.
    let Some(_lock) = super::state::try_acquire_pass_lock(global_root)? else {
        return Ok(QaSweepOutcome {
            lock_busy: true,
            ..QaSweepOutcome::default()
        });
    };

    let registry_path = workspace_registry::registry_path_for(global_root);
    let mut registry = workspace_registry::load_registry_from(&registry_path)?;
    workspace_registry::validate_workspaces(&mut registry);

    let state = load_state(&state_path(global_root));

    let reports = qa
        .workspaces
        .iter()
        .filter(|ws_config| {
            options
                .workspace
                .as_deref()
                .is_none_or(|workspace| ws_config.name == workspace)
        })
        .map(|ws_config| {
            let Some(workspace) = registry
                .workspaces
                .iter()
                .find(|candidate| candidate.name == ws_config.name)
            else {
                // Loud: a configured-but-unregistered workspace is a
                // misconfiguration, not a benign skip.
                return QaWorkspaceReport {
                    reason: Some("workspace not found in the global registry".to_string()),
                    ..QaWorkspaceReport::bare(&ws_config.name, "error")
                };
            };
            let baseline = state
                .workspaces
                .get(&ws_config.name)
                .map(|watermark| watermark.last_validated_sha.clone());
            sweep_workspace(
                global_root,
                &qa,
                ws_config,
                workspace,
                baseline,
                options.dry_run,
            )
            .unwrap_or_else(|error| QaWorkspaceReport {
                reason: Some(error.to_string()),
                ..QaWorkspaceReport::bare(&ws_config.name, "error")
            })
        })
        .collect();

    Ok(QaSweepOutcome {
        lock_busy: false,
        reports,
    })
}

fn sweep_workspace(
    global_root: &Path,
    qa: &QaSweepConfig,
    ws_config: &QaWorkspaceConfig,
    workspace: &Workspace,
    baseline: Option<String>,
    dry_run: bool,
) -> Result<QaWorkspaceReport, OrbitError> {
    if workspace.status != WorkspaceStatus::Active || !workspace.orbit_dir.exists() {
        return Ok(QaWorkspaceReport::skipped(
            &ws_config.name,
            "workspace_inactive",
        ));
    }

    let repo_root = &workspace.root;
    let branch = ws_config
        .branch
        .clone()
        .unwrap_or_else(|| workspace.base_branch.clone());

    // The sweep validates the live checkout at local HEAD (see `qa::git`); a
    // checkout parked on another branch would validate the wrong content and
    // poison the watermark, so it is skipped until it returns.
    let current_branch = git::current_branch(repo_root)?;
    if current_branch != branch {
        return Ok(QaWorkspaceReport {
            branch: Some(branch),
            ..QaWorkspaceReport::skipped(
                &ws_config.name,
                &format!("not_on_branch (checkout is on '{current_branch}')"),
            )
        });
    }

    let head = git::head_sha(repo_root)?;
    if baseline.as_deref() == Some(head.as_str()) {
        return Ok(QaWorkspaceReport {
            branch: Some(branch),
            head: Some(head.clone()),
            baseline: Some(head),
            ..QaWorkspaceReport::skipped(&ws_config.name, "no_new_commits")
        });
    }

    // `Some(None)` = the watermark commit no longer resolves (history
    // rewrite): treat the range as unknown and re-validate HEAD.
    let ranged = baseline
        .as_deref()
        .map(|from| git::commit_range(repo_root, from, &head, EVIDENCE_COMMIT_LIMIT))
        .transpose()?;
    let watermark_reset = matches!(ranged, Some(None));
    let new_commits = ranged.flatten();

    let mut report = QaWorkspaceReport {
        branch: Some(branch.clone()),
        head: Some(head.clone()),
        baseline: baseline.clone(),
        new_commits,
        watermark_reset,
        ..QaWorkspaceReport::bare(&ws_config.name, "validated")
    };

    if dry_run {
        report.action = "would_validate";
        report.checks = ws_config
            .checks
            .iter()
            .map(|check| check_report(check, if check.mute { "muted" } else { "would_run" }, 0))
            .collect();
        return Ok(report);
    }

    let runtime = OrbitRuntime::from_roots(global_root, &workspace.orbit_dir)?;
    let dirty = git::is_dirty(repo_root)?;
    let started_at = Utc::now();
    let started = Instant::now();

    let input = json!({
        "trigger": "qa-sweep",
        "workspace": ws_config.name,
        "branch": branch,
        "baseline": baseline,
        "head": head,
        "new_commits": report.new_commits.as_ref().map(Vec::len),
        "watermark_reset": watermark_reset,
        "dirty_working_tree": dirty,
        "checks": ws_config.checks.iter().map(|check| json!({
            "name": check.name,
            "muted": check.mute,
        })).collect::<Vec<_>>(),
    });

    let run = runtime
        .stores()
        .jobs()
        .insert_run(QA_SWEEP_JOB, 1, started_at, Some(input), None)?;
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, std::process::id())?;
    runtime.record_event(OrbitEvent::JobRunStarted {
        job_id: QA_SWEEP_JOB.to_string(),
        run_id: run.run_id.clone(),
        attempt: 1,
    })?;
    report.run_id = Some(run.run_id.clone());

    let all_green = match run_workspace_checks(&runtime, qa, ws_config, &run.run_id, &mut report) {
        Ok(all_green) => all_green,
        Err(error) => {
            // Don't leave the run record dangling in `running` when the pass
            // itself broke (spawn failure, store error): finalize it as
            // failed, best-effort, then surface the error as this
            // workspace's report row.
            let _ = runtime.stores().jobs().finalize_run(
                &run.run_id,
                JobRunState::Failed,
                Utc::now(),
                Some(elapsed_ms(started)),
            );
            let _ = runtime.record_event(OrbitEvent::JobRunCompleted {
                job_id: QA_SWEEP_JOB.to_string(),
                run_id: run.run_id.clone(),
                state: JobRunState::Failed.to_string(),
            });
            return Err(error);
        }
    };

    let final_state = if all_green {
        JobRunState::Success
    } else {
        JobRunState::Failed
    };
    let finished_at = Utc::now();
    runtime.stores().jobs().finalize_run(
        &run.run_id,
        final_state,
        finished_at,
        Some(elapsed_ms(started)),
    )?;
    runtime.record_event(OrbitEvent::JobRunCompleted {
        job_id: QA_SWEEP_JOB.to_string(),
        run_id: run.run_id.clone(),
        state: final_state.to_string(),
    })?;

    if all_green {
        // Advance to the sha the checks actually ran against; commits landing
        // mid-pass are picked up by the next sweep.
        advance_watermark(
            &state_path(global_root),
            &ws_config.name,
            QaWorkspaceWatermark {
                last_validated_sha: head,
                validated_at: finished_at.to_rfc3339(),
                run_id: Some(run.run_id),
            },
        )?;
    } else {
        report.action = "failed";
    }

    Ok(report)
}

/// Execute the workspace's checks against the checkout, recording one ledger
/// step per executed check and filing/deduping tasks for failures. Returns
/// whether every executed (non-muted) check passed.
fn run_workspace_checks(
    runtime: &OrbitRuntime,
    qa: &QaSweepConfig,
    ws_config: &QaWorkspaceConfig,
    run_id: &str,
    report: &mut QaWorkspaceReport,
) -> Result<bool, OrbitError> {
    let repo_root = runtime.paths().repo_root.clone();
    let mut all_green = true;
    let mut step_index = 0usize;
    for check in &ws_config.checks {
        if check.mute {
            report.checks.push(check_report(check, "muted", 0));
            continue;
        }

        let step_started_at = Utc::now();
        let execution = execute_check(&repo_root, &check.command, check.timeout)?;
        let step_finished_at = Utc::now();

        let mut entry = check_report(
            check,
            match (execution.timed_out, execution.exit_code) {
                (true, _) => "timeout",
                (false, Some(0)) => "passed",
                (false, _) => "failed",
            },
            execution.duration_ms,
        );
        entry.exit_code = execution.exit_code;
        let passed = entry.outcome == "passed";
        all_green &= passed;

        if !passed {
            let finding = handle_failure(runtime, qa, ws_config, check, report, &execution)?;
            entry.fingerprint = Some(finding.fingerprint);
            entry.filed_task = finding.filed_task;
            entry.deduped_task = finding.deduped_task;
        }

        runtime.stores().jobs().complete_run_step(
            run_id,
            &JobRunStepParams {
                step_index,
                target_type: JobTargetType::Job,
                target_id: format!("{QA_SWEEP_JOB}:{}", check.name),
                started_at: step_started_at,
                finished_at: step_finished_at,
                duration_ms: Some(execution.duration_ms),
                exit_code: execution.exit_code,
                agent_response_json: Some(step_summary_json(&entry)),
                state: if passed {
                    JobRunState::Success
                } else {
                    JobRunState::Failed
                },
                error_code: None,
                error_message: (!passed).then(|| {
                    format!(
                        "check '{}' {}: {}",
                        check.name,
                        entry.outcome,
                        evidence_excerpt(&execution.output)
                    )
                }),
            },
        )?;
        step_index += 1;
        report.checks.push(entry);
    }
    Ok(all_green)
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn check_report(check: &QaCheck, outcome: &'static str, duration_ms: u64) -> QaCheckReport {
    QaCheckReport {
        name: check.name.clone(),
        outcome,
        exit_code: None,
        duration_ms,
        fingerprint: None,
        filed_task: None,
        deduped_task: None,
    }
}

fn step_summary_json(entry: &QaCheckReport) -> Value {
    json!({
        "check": entry.name,
        "outcome": entry.outcome,
        "fingerprint": entry.fingerprint,
        "filed_task": entry.filed_task,
        "deduped_task": entry.deduped_task,
    })
}

struct FailureFinding {
    fingerprint: String,
    filed_task: Option<String>,
    deduped_task: Option<String>,
}

/// Fingerprint a failing check, dedupe against open qa-sweep tasks carrying
/// the same fingerprint tag, and file a task when none exists.
fn handle_failure(
    runtime: &OrbitRuntime,
    qa: &QaSweepConfig,
    ws_config: &QaWorkspaceConfig,
    check: &QaCheck,
    report: &QaWorkspaceReport,
    execution: &CheckExecution,
) -> Result<FailureFinding, OrbitError> {
    let exit_summary = if execution.timed_out {
        format!("timeout after {}s", check.timeout.as_secs())
    } else {
        match execution.exit_code {
            Some(code) => format!("exit {code}"),
            None => "killed by signal".to_string(),
        }
    };
    let repo_root = runtime.paths().repo_root.display().to_string();
    let fingerprint = failure_fingerprint(
        &ws_config.name,
        &check.name,
        &repo_root,
        &execution.output,
        &exit_summary,
    );
    let tags = vec![QA_SWEEP_TAG.to_string(), fingerprint_tag(&fingerprint)];

    let open_task = runtime
        .list_tasks_by_tags(&tags)?
        .into_iter()
        .find(task_is_open);
    if let Some(task) = open_task {
        return Ok(FailureFinding {
            fingerprint,
            filed_task: None,
            deduped_task: Some(task.id),
        });
    }

    let task = runtime.add_task(TaskAddParams {
        title: format!(
            "qa-sweep: check '{}' failing in {}",
            check.name, ws_config.name
        ),
        description: failure_description(ws_config, check, report, execution, &exit_summary),
        acceptance_criteria: vec![format!(
            "`{}` exits 0 from the {} workspace root on `{}`",
            check.command,
            ws_config.name,
            report.branch.as_deref().unwrap_or("agent-main"),
        )],
        tags,
        priority: check.priority.unwrap_or(qa.default_priority),
        task_type: Some(TaskType::Bug),
        status: Some(qa.task_status),
        system_created: true,
        ..TaskAddParams::default()
    })?;

    Ok(FailureFinding {
        fingerprint,
        filed_task: Some(task.id),
        deduped_task: None,
    })
}

fn task_is_open(task: &Task) -> bool {
    !matches!(
        task.status,
        TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
    )
}

fn failure_description(
    ws_config: &QaWorkspaceConfig,
    check: &QaCheck,
    report: &QaWorkspaceReport,
    execution: &CheckExecution,
    exit_summary: &str,
) -> String {
    let range = match (&report.baseline, report.watermark_reset) {
        (Some(baseline), false) => format!(
            "{}..{} ({} new commit(s) shown below)",
            baseline,
            report.head.as_deref().unwrap_or("HEAD"),
            report.new_commits.as_ref().map(Vec::len).unwrap_or(0),
        ),
        (Some(baseline), true) => format!(
            "unknown — last-validated commit {baseline} no longer resolves (history rewrite); validated HEAD {}",
            report.head.as_deref().unwrap_or("HEAD"),
        ),
        (None, _) => format!(
            "first validation of this workspace; validated HEAD {}",
            report.head.as_deref().unwrap_or("HEAD"),
        ),
    };
    let commits = report
        .new_commits
        .as_ref()
        .filter(|commits| !commits.is_empty())
        .map(|commits| format!("\nCommits since last green:\n{}\n", commits.join("\n")))
        .unwrap_or_default();

    format!(
        "Automated qa-sweep finding.\n\n\
         - workspace: {workspace}\n\
         - check: {check_name} (`sh -c {command}`)\n\
         - branch: {branch}\n\
         - result: {exit_summary}\n\
         - commit range since last green: {range}\n\
         - ledger run: {run} (`orbit run show <run_id>` in the workspace)\n\
         {commits}\n\
         Output (head):\n```text\n{output}\n```\n",
        workspace = ws_config.name,
        check_name = check.name,
        command = check.command,
        branch = report.branch.as_deref().unwrap_or("agent-main"),
        run = report.run_id.as_deref().unwrap_or("not recorded"),
        output = evidence_excerpt(&execution.output),
    )
}

/// Head of the combined output, bounded in lines and bytes.
fn evidence_excerpt(output: &str) -> String {
    let mut excerpt = output
        .lines()
        .take(EVIDENCE_OUTPUT_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if excerpt.len() > EVIDENCE_OUTPUT_BYTES {
        let mut cut = EVIDENCE_OUTPUT_BYTES;
        while !excerpt.is_char_boundary(cut) {
            cut -= 1;
        }
        excerpt.truncate(cut);
    }
    if excerpt.trim().is_empty() {
        "(no output)".to_string()
    } else {
        excerpt
    }
}

pub(super) struct CheckExecution {
    pub(super) exit_code: Option<i32>,
    pub(super) timed_out: bool,
    pub(super) output: String,
    pub(super) duration_ms: u64,
}

/// Run one check via `sh -c` from the workspace root, killing it once the
/// configured timeout elapses. stdout and stderr are captured off-thread (so
/// a chatty check cannot deadlock on a full pipe) and concatenated
/// stdout-then-stderr for evidence and fingerprinting.
///
/// On Unix the check runs in its own session/process group and a timeout
/// kills the whole group — otherwise grandchildren (a `make` fanning out to
/// compilers, a shell forking `sleep`) would survive the kill and keep the
/// output pipes open, wedging the sweep until they exit on their own.
pub(super) fn execute_check(
    repo_root: &Path,
    command: &str,
    timeout: Duration,
) -> Result<CheckExecution, OrbitError> {
    let started = Instant::now();
    let mut builder = Command::new("sh");
    builder
        .arg("-c")
        .arg(command)
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        builder.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = builder.spawn().map_err(|error| {
        OrbitError::Execution(format!("spawn check `sh -c {command}`: {error}"))
    })?;

    let stdout = drain_off_thread(child.stdout.take());
    let stderr = drain_off_thread(child.stderr.take());

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    kill_check_group(&mut child);
                    break child.wait().ok();
                }
                std::thread::sleep(CHECK_POLL_INTERVAL);
            }
            Err(error) => {
                return Err(OrbitError::Execution(format!(
                    "wait on check `{command}`: {error}"
                )));
            }
        }
    };

    let mut output = stdout.join().unwrap_or_default();
    let err_output = stderr.join().unwrap_or_default();
    if !err_output.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&err_output);
    }

    Ok(CheckExecution {
        exit_code: if timed_out {
            None
        } else {
            status.and_then(|status| status.code())
        },
        timed_out,
        output,
        duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

/// Kill a timed-out check together with its process group (Unix) so orphaned
/// grandchildren cannot hold the captured pipes open.
fn kill_check_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        // The check was made its own session leader in `pre_exec`, so its pid
        // is the process-group id; negative pid targets the whole group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

fn drain_off_thread<R: std::io::Read + Send + 'static>(
    source: Option<R>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut source) = source {
            let _ = std::io::Read::read_to_end(&mut source, &mut buffer);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

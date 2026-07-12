//! `orbit run qa-sweep` — the trailing QA pass over direct-push workspaces
//! [ORB-10039], reworked to a worker-invoked QA agent pass [ORB-10146].
//!
//! Direct pushes to `agent-main` stay fast at write time; this sweep validates
//! them on a lag. Per configured workspace it diffs the live checkout's HEAD
//! against a per-workspace last-validated watermark and, when new commits
//! exist, submits a **QA agent run** to the worker invoke daemon: the agent
//! reads the new commits, exercises the new features/behaviour changes
//! hands-on, and emits a structured findings report. The sweep parses that
//! report, files fingerprint-deduped orbit tasks for the findings, and advances
//! the watermark whenever the run completed and the report parsed (findings are
//! captured as tasks, so re-validating the same range adds nothing). A failed,
//! timed-out, or unparseable run leaves the watermark and is reported as an
//! `error` row — never a silent green.
//!
//! **Ledger integration.** Every validating pass records a first-class v2 job
//! run (job id [`QA_SWEEP_JOB`]) in the swept workspace's jobs store — one run
//! per workspace per pass, with a single run step whose payload links the
//! worker `run_id` for the agent run — so `orbit run history -j qa_sweep` and
//! `orbit run show <run_id>` surface the sweeps honestly.
//!
//! Like ship-sweep, this never bootstraps a `.orbit/` in the scheduler's cwd:
//! everything resolves from the global registry and global config, and
//! per-workspace failures are isolated into report rows.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{
    Crew, JobRunState, JobTargetType, OrbitError, OrbitEvent, Task, TaskStatus, TaskType,
    Workspace, WorkspaceStatus,
};
use orbit_store::JobRunStepParams;
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;
use crate::config::RuntimeConfig;
use crate::workspace_registry;

use super::config::{QaSweepConfig, QaWorkspaceConfig};
use super::fingerprint::{QA_SWEEP_TAG, finding_fingerprint, fingerprint_tag};
use super::git;
use super::prompt::{PromptInputs, compose_prompt};
use super::report::{QaReport, parse_report, resolve_priority};
use super::state::{QaWorkspaceWatermark, advance_watermark, load_state, state_path};
use super::worker::{STATUS_OK, WorkerClient, WorkerError, WorkerRunRequest};

/// Job id qa-sweep runs are recorded under in each workspace's run ledger.
pub const QA_SWEEP_JOB: &str = "qa_sweep";

/// Commits listed in the QA prompt / as evidence, when a per-workspace
/// `max_commits` cap is not set.
const EVIDENCE_COMMIT_LIMIT: usize = 30;
/// Evidence text quoted per filed task.
const EVIDENCE_TEXT_LINES: usize = 60;
const EVIDENCE_TEXT_BYTES: usize = 6000;
/// Turn budget for a QA agent run (providers that support the control).
const QA_MAX_TURNS: u32 = 150;

/// Options for one qa-sweep pass.
#[derive(Debug, Clone, Default)]
pub struct QaSweepOptions {
    /// Report what would be validated without invoking the agent, recording
    /// runs, filing tasks, or advancing watermarks.
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
    /// One of: `validated`, `error`, `would_validate`, `skipped`.
    pub action: &'static str,
    /// Why, for `skipped` / `error` rows.
    pub reason: Option<String>,
    /// Branch the checkout was validated on.
    pub branch: Option<String>,
    /// Resolved crew name for the QA agent run.
    pub crew: Option<String>,
    /// HEAD sha the agent validated against.
    pub head: Option<String>,
    /// Watermark sha the diff was computed from (`None` = first validation).
    pub baseline: Option<String>,
    /// Commits in `baseline..head`, capped. `None` when the baseline no longer
    /// resolves (history rewrite).
    pub new_commits: Option<Vec<String>>,
    /// True when the watermark existed but no longer resolves in the repo.
    pub watermark_reset: bool,
    /// Ledger run id, when a run was recorded.
    pub run_id: Option<String>,
    /// Worker agent run id, when one was created.
    pub agent_run_id: Option<String>,
    /// Findings filed/deduped for a validating pass.
    pub findings: Vec<QaFindingReport>,
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
            crew: None,
            head: None,
            baseline: None,
            new_commits: None,
            watermark_reset: false,
            run_id: None,
            agent_run_id: None,
            findings: Vec::new(),
        }
    }
}

/// One finding's disposition within a validating pass.
#[derive(Debug)]
pub struct QaFindingReport {
    /// Finding name as reported by the agent.
    pub name: String,
    /// Reported severity (lowercase).
    pub severity: String,
    /// Dedupe fingerprint.
    pub fingerprint: String,
    /// Task id filed for this finding, when one was created this pass.
    pub filed_task: Option<String>,
    /// Open task id the finding deduped against, when one already existed.
    pub deduped_task: Option<String>,
}

/// The QA agent invocation seam. Production submits to the worker daemon; tests
/// inject a fake to exercise report parsing, dedupe, watermark rules, and
/// worker-client failure paths without a live daemon.
pub(crate) trait QaAgent {
    fn run(&self, request: QaAgentRequest) -> Result<QaAgentRun, QaAgentError>;
}

/// A QA agent run request, resolved from a workspace's config + checkout.
#[derive(Debug, Clone)]
pub(crate) struct QaAgentRequest {
    pub workspace: String,
    pub repo_root: PathBuf,
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub timeout: Duration,
}

/// A QA agent run that reached a terminal state.
#[derive(Debug, Clone)]
pub(crate) struct QaAgentRun {
    /// Worker run id.
    pub agent_run_id: String,
    /// Terminal status string.
    pub status: String,
    /// Final agent output text.
    pub report_text: Option<String>,
}

/// A QA agent run that could not be completed. Carries the worker run id when
/// one was created (e.g. a timeout) so the ledger step can still link it.
#[derive(Debug, Clone)]
pub(crate) struct QaAgentError {
    pub agent_run_id: Option<String>,
    pub message: String,
}

/// Production [`QaAgent`] backed by the loopback worker invoke daemon.
pub(crate) struct WorkerQaAgent {
    base_url: String,
}

impl WorkerQaAgent {
    pub(crate) fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }
}

impl QaAgent for WorkerQaAgent {
    fn run(&self, request: QaAgentRequest) -> Result<QaAgentRun, QaAgentError> {
        let client = WorkerClient::new(&self.base_url).map_err(|error| QaAgentError {
            agent_run_id: None,
            message: error.to_string(),
        })?;
        // Only providers with a turn control get `max_turns`; others (Codex
        // rejects the control outright) rely on the wall-clock budget.
        let max_turns = (request.provider == "claude").then_some(QA_MAX_TURNS);
        let worker_request = WorkerRunRequest {
            prompt: request.prompt,
            provider: request.provider,
            model: request.model,
            cwd: request.repo_root.display().to_string(),
            wall_clock_secs: request.timeout.as_secs().max(1),
            max_turns,
            serialization_key: Some(format!("qa-sweep:{}", request.workspace)),
        };
        match client.run_to_terminal(&worker_request, request.timeout) {
            Ok((run_id, terminal)) => Ok(QaAgentRun {
                agent_run_id: run_id,
                status: terminal.status,
                report_text: terminal.report_text,
            }),
            Err(WorkerError::Timeout {
                run_id,
                waited_secs,
            }) => Err(QaAgentError {
                agent_run_id: Some(run_id),
                message: format!("agent run timed out after {waited_secs}s"),
            }),
            Err(other) => Err(QaAgentError {
                agent_run_id: None,
                message: other.to_string(),
            }),
        }
    }
}

/// Run one qa-sweep pass against the default global root (`~/.orbit`).
pub fn run_qa_sweep(options: QaSweepOptions) -> Result<QaSweepOutcome, OrbitError> {
    let global_root = workspace_registry::global_orbit_dir()?;
    run_qa_sweep_at(&global_root, options)
}

/// Run one qa-sweep pass against an explicit global root, invoking the real
/// worker daemon.
pub fn run_qa_sweep_at(
    global_root: &Path,
    options: QaSweepOptions,
) -> Result<QaSweepOutcome, OrbitError> {
    // The worker base URL is read from the same host-level `[qa]` config the
    // rest of the pass uses; build the agent up front so a bad URL fails once.
    let config = RuntimeConfig::load_layered(global_root, global_root)?;
    let base_url = config.qa_sweep().worker_base_url.clone();
    let agent = WorkerQaAgent::new(&base_url);
    run_qa_sweep_with(global_root, options, &agent)
}

/// Run one qa-sweep pass with an injected [`QaAgent`] (test seam).
pub(crate) fn run_qa_sweep_with(
    global_root: &Path,
    options: QaSweepOptions,
    agent: &dyn QaAgent,
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
                agent,
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

#[allow(clippy::too_many_arguments)]
fn sweep_workspace(
    global_root: &Path,
    qa: &QaSweepConfig,
    ws_config: &QaWorkspaceConfig,
    workspace: &Workspace,
    baseline: Option<String>,
    dry_run: bool,
    agent: &dyn QaAgent,
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
    let commit_limit = ws_config.max_commits.unwrap_or(EVIDENCE_COMMIT_LIMIT);
    let ranged = baseline
        .as_deref()
        .map(|from| git::commit_range(repo_root, from, &head, commit_limit))
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

    // Resolve the crew for the QA run: the per-workspace `crew` override, else
    // the workspace's default crew resolution [ORB-10133]. Done against the
    // workspace's own runtime so its crew registry / default apply.
    let runtime = OrbitRuntime::from_roots(global_root, &workspace.orbit_dir)?;
    let crew = runtime.resolve_crew_for_task(ws_config.crew.as_deref(), None)?;
    report.crew = Some(crew.name.clone());

    if dry_run {
        report.action = "would_validate";
        return Ok(report);
    }

    let prompt = compose_prompt(&PromptInputs {
        workspace: &ws_config.name,
        repo_root: &repo_root.display().to_string(),
        branch: &branch,
        baseline: baseline.as_deref(),
        head: &head,
        watermark_reset,
        commits: report.new_commits.as_deref().unwrap_or_default(),
    });

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
        "crew": crew.name,
        "provider": crew.assignment.provider,
        "model": crew.assignment.model,
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

    // Invoke the QA agent. Errors here are pass outcomes, not sweep aborts: the
    // ledger run is always finalized so no run dangles in `running`.
    let outcome = agent.run(QaAgentRequest {
        workspace: ws_config.name.clone(),
        repo_root: repo_root.clone(),
        provider: crew.assignment.provider.clone(),
        model: crew.assignment.model.clone(),
        prompt,
        timeout: ws_config.timeout,
    });
    report.agent_run_id = match &outcome {
        Ok(run) => Some(run.agent_run_id.clone()),
        Err(error) => error.agent_run_id.clone(),
    };
    let terminal_status = match &outcome {
        Ok(run) => run.status.clone(),
        Err(_) => "no_run".to_string(),
    };

    let finalize_failed = |error: OrbitError| -> OrbitError {
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
        error
    };

    let classified = classify_outcome(&outcome);
    let (success, reason) = match &classified {
        Ok(qa_report) => {
            // A store error while filing is a genuine sweep failure: finalize the
            // run failed and surface it as this workspace's error row.
            let findings = file_findings(&runtime, qa, ws_config, &report, qa_report)
                .map_err(finalize_failed)?;
            report.findings = findings;
            (true, None)
        }
        Err(reason) => (false, Some(reason.clone())),
    };

    record_agent_step(
        &runtime,
        &run.run_id,
        started_at,
        started,
        &crew,
        &report,
        &terminal_status,
        success,
        reason.as_deref(),
    )?;

    let final_state = if success {
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

    if success {
        // Findings are captured as tasks, so re-validating this range adds
        // nothing: advance to the sha the agent ran against. Commits landing
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
        report.action = "validated";
    } else {
        report.action = "error";
        report.reason = reason;
    }

    Ok(report)
}

/// Classify the agent outcome into a parsed report (advance the watermark) or a
/// failure reason (hold it). Only an `ok` terminal run whose output carries a
/// parseable `findings` report counts as a completed validation.
fn classify_outcome(outcome: &Result<QaAgentRun, QaAgentError>) -> Result<QaReport, String> {
    match outcome {
        Ok(run) if run.status == STATUS_OK => {
            let text = run.report_text.as_deref().unwrap_or("");
            parse_report(text).map_err(|error| format!("unparseable findings report: {error}"))
        }
        Ok(run) => Err(format!(
            "agent run ended in non-success state '{}'",
            run.status
        )),
        Err(error) => Err(error.message.clone()),
    }
}

/// File a fingerprint-deduped task per finding, returning the per-finding
/// disposition (filed vs deduped against an existing open task).
fn file_findings(
    runtime: &OrbitRuntime,
    qa: &QaSweepConfig,
    ws_config: &QaWorkspaceConfig,
    report: &QaWorkspaceReport,
    qa_report: &QaReport,
) -> Result<Vec<QaFindingReport>, OrbitError> {
    let mut filed = Vec::new();
    for finding in &qa_report.findings {
        let fingerprint = finding_fingerprint(&ws_config.name, &finding.name);
        let tags = vec![QA_SWEEP_TAG.to_string(), fingerprint_tag(&fingerprint)];

        let open_task = runtime
            .list_tasks_by_tags(&tags)?
            .into_iter()
            .find(task_is_open);
        if let Some(task) = open_task {
            filed.push(QaFindingReport {
                name: finding.name.clone(),
                severity: finding.severity.as_str().to_string(),
                fingerprint,
                filed_task: None,
                deduped_task: Some(task.id),
            });
            continue;
        }

        let priority = resolve_priority(finding.severity, qa.default_priority);
        let task = runtime.add_task(TaskAddParams {
            title: finding_title(ws_config, &finding.name),
            description: finding_description(ws_config, report, finding),
            acceptance_criteria: vec![format!(
                "The qa-sweep finding '{}' in {} on `{}` is resolved — the new behaviour works as \
                 intended (verified hands-on, not just a green test suite).",
                finding.name,
                ws_config.name,
                report.branch.as_deref().unwrap_or("agent-main"),
            )],
            tags,
            priority,
            task_type: Some(TaskType::Bug),
            status: Some(qa.task_status),
            system_created: true,
            ..TaskAddParams::default()
        })?;

        filed.push(QaFindingReport {
            name: finding.name.clone(),
            severity: finding.severity.as_str().to_string(),
            fingerprint,
            filed_task: Some(task.id),
            deduped_task: None,
        });
    }
    Ok(filed)
}

/// Record the single agent run step in the ledger, linking the worker run id.
#[allow(clippy::too_many_arguments)]
fn record_agent_step(
    runtime: &OrbitRuntime,
    run_id: &str,
    started_at: chrono::DateTime<Utc>,
    started: Instant,
    crew: &Crew,
    report: &QaWorkspaceReport,
    terminal_status: &str,
    success: bool,
    reason: Option<&str>,
) -> Result<(), OrbitError> {
    let payload = json!({
        "agent_run_id": report.agent_run_id,
        "worker_status": terminal_status,
        "crew": crew.name,
        "provider": crew.assignment.provider,
        "model": crew.assignment.model,
        "findings": report.findings.len(),
        "filed": report.findings.iter().filter(|f| f.filed_task.is_some()).count(),
        "deduped": report.findings.iter().filter(|f| f.deduped_task.is_some()).count(),
    });
    runtime.stores().jobs().complete_run_step(
        run_id,
        &JobRunStepParams {
            step_index: 0,
            target_type: JobTargetType::Job,
            target_id: format!("{QA_SWEEP_JOB}:agent"),
            started_at,
            finished_at: Utc::now(),
            duration_ms: Some(elapsed_ms(started)),
            exit_code: None,
            agent_response_json: Some(payload),
            state: if success {
                JobRunState::Success
            } else {
                JobRunState::Failed
            },
            error_code: None,
            error_message: reason.map(|reason| {
                format!(
                    "qa agent run for {} did not validate: {reason}",
                    report.workspace
                )
            }),
        },
    )?;
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn task_is_open(task: &Task) -> bool {
    !matches!(
        task.status,
        TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
    )
}

/// A concise task title from a finding name (bounded, single line).
fn finding_title(ws_config: &QaWorkspaceConfig, name: &str) -> String {
    let clean = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = if clean.is_empty() {
        "unnamed QA finding".to_string()
    } else {
        clean
    };
    let mut title = format!("qa-sweep: {clean} ({})", ws_config.name);
    if title.len() > 120 {
        let mut cut = 117;
        while !title.is_char_boundary(cut) {
            cut -= 1;
        }
        title.truncate(cut);
        title.push_str("...");
    }
    title
}

fn finding_description(
    ws_config: &QaWorkspaceConfig,
    report: &QaWorkspaceReport,
    finding: &super::report::Finding,
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
    let finding_commits = if finding.commits.is_empty() {
        String::new()
    } else {
        format!(
            "\nAttributed commits:\n{}\n",
            finding
                .commits
                .iter()
                .map(|commit| format!("- {commit}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    format!(
        "Automated qa-sweep finding (QA agent pass).\n\n\
         - finding: {name}\n\
         - workspace: {workspace}\n\
         - branch: {branch}\n\
         - severity: {severity}\n\
         - commit range since last green: {range}\n\
         - ledger run: {run} (`orbit run show <run_id>` in the workspace)\n\
         - worker agent run: {agent_run}\n\
         {commits}{finding_commits}\n\
         Summary: {summary}\n\n\
         Evidence:\n```text\n{evidence}\n```\n",
        name = finding.name,
        workspace = ws_config.name,
        branch = report.branch.as_deref().unwrap_or("agent-main"),
        severity = finding.severity.as_str(),
        run = report.run_id.as_deref().unwrap_or("not recorded"),
        agent_run = report.agent_run_id.as_deref().unwrap_or("not recorded"),
        summary = if finding.summary.is_empty() {
            "(none provided)"
        } else {
            finding.summary.as_str()
        },
        evidence = evidence_excerpt(&finding.evidence),
    )
}

/// Head of the evidence text, bounded in lines and bytes.
fn evidence_excerpt(evidence: &str) -> String {
    let mut excerpt = evidence
        .lines()
        .take(EVIDENCE_TEXT_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if excerpt.len() > EVIDENCE_TEXT_BYTES {
        let mut cut = EVIDENCE_TEXT_BYTES;
        while !excerpt.is_char_boundary(cut) {
            cut -= 1;
        }
        excerpt.truncate(cut);
    }
    if excerpt.trim().is_empty() {
        "(no evidence provided)".to_string()
    } else {
        excerpt
    }
}

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use orbit_core::command::gc::{
    EmptyGcCollector, GcCollector, GcReport, GcRequest, GcScope, GcTarget, RunGcCollector,
    RunGcPolicy, SystemGcClock, WorktreeGcCollector, WorktreeGcPolicy, execute_gc,
};
use orbit_core::command::gc_audit::AuditGcCollector;
use orbit_core::command::gc_logs::LogsGcCollector;
use orbit_core::command::skill_gc::SkillsGcCollector;
use orbit_core::command::task_gc::TaskGcCollector;
use orbit_core::{OrbitError, OrbitRuntime};

use super::Execute;
use crate::parse::parse_duration_seconds;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GcTargetArg {
    Worktrees,
    Runs,
    Logs,
    Diagnostics,
    Audit,
    Skills,
    Tasks,
    All,
}

impl From<GcTargetArg> for GcTarget {
    fn from(value: GcTargetArg) -> Self {
        match value {
            GcTargetArg::Worktrees => Self::Worktrees,
            GcTargetArg::Runs => Self::Runs,
            GcTargetArg::Logs => Self::Logs,
            GcTargetArg::Diagnostics => Self::Diagnostics,
            GcTargetArg::Audit => Self::Audit,
            GcTargetArg::Skills => Self::Skills,
            GcTargetArg::Tasks => Self::Tasks,
            GcTargetArg::All => Self::All,
        }
    }
}

#[derive(Debug, Args)]
#[command(about = "Plan or apply garbage collection for Orbit-owned state")]
pub struct GcCommand {
    /// Storage family to inspect (omitted means all available targets)
    #[arg(value_enum, default_value_t = GcTargetArg::All)]
    pub target: GcTargetArg,

    /// Perform the mutations in the frozen plan
    #[arg(long)]
    pub apply: bool,

    /// Emit the complete versioned report as JSON
    #[arg(long)]
    pub json: bool,

    /// Override the target's retention for this invocation
    #[arg(long, value_name = "DURATION")]
    pub retention: Option<String>,

    /// Also archive `rejected` tasks (the `tasks` target only; `done` is always
    /// eligible). Rejected by every other target.
    #[arg(long)]
    pub include_rejected: bool,

    /// Override successful/cancelled worktree retention in days
    #[arg(long, value_name = "DAYS")]
    pub success_retention_days: Option<u64>,

    /// Override failed/timeout/interrupted worktree retention in days
    #[arg(long, value_name = "DAYS")]
    pub failure_retention_days: Option<u64>,

    /// Override successful/cancelled run archive age in days
    #[arg(long, value_name = "DAYS")]
    pub archive_after_days: Option<u64>,

    /// Override successful/cancelled run purge age in days
    #[arg(long, value_name = "DAYS")]
    pub purge_after_days: Option<u64>,

    /// Override failed/timeout/interrupted run archive age in days
    #[arg(long, value_name = "DAYS")]
    pub failure_archive_after_days: Option<u64>,

    /// Override failed/timeout/interrupted run purge age in days
    #[arg(long, value_name = "DAYS")]
    pub failure_purge_after_days: Option<u64>,

    /// Select the current registered workspace by ID or path
    #[arg(long, value_name = "ID_OR_PATH", conflicts_with = "global")]
    pub workspace: Option<String>,

    /// Select global Orbit-owned state only
    #[arg(long, conflicts_with = "workspace")]
    pub global: bool,
}

impl Execute for GcCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let target = GcTarget::from(self.target);
        // Runs retention is workspace-only per the normative GC design (§3.2):
        // reject `--global` before constructing a runtime or planning so an
        // unsupported scope refuses rather than scanning/mutating the wrong
        // (global) state tree.
        if self.global && target == GcTarget::Runs {
            return Err(OrbitError::InvalidInput(
                "run garbage collection is workspace-only; `--global` is not a supported scope"
                    .to_string(),
            ));
        }
        if target != GcTarget::Worktrees
            && (self.success_retention_days.is_some() || self.failure_retention_days.is_some())
        {
            return Err(OrbitError::InvalidInput(
                "worktree retention overrides require the `worktrees` target".to_string(),
            ));
        }
        let has_run_override = self.archive_after_days.is_some()
            || self.purge_after_days.is_some()
            || self.failure_archive_after_days.is_some()
            || self.failure_purge_after_days.is_some();
        if target != GcTarget::Runs && has_run_override {
            return Err(OrbitError::InvalidInput(
                "run retention overrides require the `runs` target".to_string(),
            ));
        }
        // `--include-rejected` toggles the optional terminal status for task
        // archival and has no meaning elsewhere. Reject it for every other
        // target up front, before any runtime construction or mutation, so an
        // operator mistake refuses rather than silently no-ops.
        if target != GcTarget::Tasks && self.include_rejected {
            return Err(OrbitError::InvalidInput(
                "the `--include-rejected` flag requires the `tasks` target".to_string(),
            ));
        }
        // Skills is a global-only collector whose owned state spans the global
        // generated-skill root and the per-agent link roots; it resolves its own
        // scope rather than following the generic workspace/global split. An
        // explicit `--workspace` selector has no meaning here, so reject it up
        // front rather than silently performing a global GC. The default and an
        // explicit `--global` both continue to select this collector.
        if target == GcTarget::Skills {
            if self.workspace.is_some() {
                return Err(OrbitError::InvalidInput(
                    "`gc skills` operates on global Orbit-owned state; the `--workspace` \
                     selector is not supported"
                        .to_string(),
                ));
            }
            let collector = SkillsGcCollector::for_global_root(&runtime.paths().global_dir);
            let scope = collector.scope();
            let report = self.run(&collector, scope, runtime)?;
            return self.finish(report);
        }
        let defaults_global = matches!(target, GcTarget::Logs);
        let scope = if self.global || (self.workspace.is_none() && defaults_global) {
            GcScope::Global {
                root: runtime.paths().global_dir.clone(),
            }
        } else {
            resolve_workspace_scope(self.workspace.as_deref(), runtime)?
        };
        // Audit is a workspace-scoped collector; it resolves the concrete
        // workspace id under its scope and runs the AuditGcCollector, which
        // holds the workspace audit writer/GC guard across mark→delete
        // (ORB-10186). A global scope has no audit root to collect, so reject it.
        if target == GcTarget::Audit {
            if matches!(scope, GcScope::Global { .. }) {
                return Err(OrbitError::InvalidInput(
                    "audit GC requires a workspace; use --workspace <id-or-path>".to_string(),
                ));
            }
            let workspace_id = match &scope {
                GcScope::Workspace {
                    workspace_id: Some(id),
                    ..
                } => id.clone(),
                GcScope::Workspace { .. } => runtime.workspace_id()?,
                GcScope::Global { .. } => unreachable!("global audit scope rejected above"),
            };
            let collector =
                AuditGcCollector::new(runtime.sqlite_store()?, workspace_id, scope.root());
            let report = self.run(&collector, scope, runtime)?;
            return self.finish(report);
        }
        // Task archival delegates to the ordinary task lifecycle, so its
        // collector borrows the workspace runtime rather than scanning a
        // filesystem root. It is workspace-scoped only (cross-workspace/global
        // selection is reserved for a future aggregate operator surface), and
        // `--include-rejected` widens the terminal set from `done`-only to also
        // include `rejected`. Handle it here as a self-contained collector.
        if target == GcTarget::Tasks {
            let scope = ensure_current_workspace_scope(scope, runtime)?;
            let collector = TaskGcCollector::new(runtime).include_rejected(self.include_rejected);
            let report = self.run(&collector, scope, runtime)?;
            return self.finish(report);
        }
        let selected_runtime = if matches!(target, GcTarget::Worktrees | GcTarget::Runs)
            && scope.root() != runtime.paths().orbit_dir
        {
            Some(OrbitRuntime::from_roots(
                &runtime.paths().global_dir,
                scope.root(),
            )?)
        } else {
            None
        };
        let collector_runtime = selected_runtime.as_ref().unwrap_or(runtime);
        let worktree_policy = WorktreeGcPolicy {
            success_retention_days: self
                .success_retention_days
                .unwrap_or_else(|| collector_runtime.worktree_gc_success_retention_days()),
            failure_retention_days: self
                .failure_retention_days
                .unwrap_or_else(|| collector_runtime.worktree_gc_failure_retention_days()),
        };
        let worktrees = WorktreeGcCollector::new(collector_runtime, worktree_policy);
        let (archive, purge, failure_archive, failure_purge) =
            collector_runtime.run_gc_retention_days();
        let runs = RunGcCollector::new(
            collector_runtime,
            RunGcPolicy {
                archive_after_days: self.archive_after_days.unwrap_or(archive),
                purge_after_days: self.purge_after_days.unwrap_or(purge),
                failure_archive_after_days: self
                    .failure_archive_after_days
                    .unwrap_or(failure_archive),
                failure_purge_after_days: self.failure_purge_after_days.unwrap_or(failure_purge),
            },
        );
        // `logs` has a real collector too; remaining targets keep the framework
        // placeholder until their domain collectors land.
        let retention_window = self
            .retention
            .as_deref()
            .map(parse_duration_seconds)
            .transpose()?
            .map(Duration::from_secs);
        let logs = matches!(target, GcTarget::Logs)
            .then(|| LogsGcCollector::from_scope(&scope, retention_window));
        let empty = EmptyGcCollector::new(target);
        let collector: &dyn GcCollector = if target == GcTarget::Worktrees {
            &worktrees
        } else if target == GcTarget::Runs {
            &runs
        } else if let Some(logs) = logs.as_ref() {
            logs
        } else {
            &empty
        };
        let report = self.run(collector, scope, runtime)?;
        self.finish(report)
    }
}

impl GcCommand {
    fn run(
        &self,
        collector: &dyn GcCollector,
        scope: GcScope,
        runtime: &OrbitRuntime,
    ) -> Result<GcReport, OrbitError> {
        let clock = SystemGcClock;
        execute_gc(
            collector,
            GcRequest {
                apply: self.apply,
                scope,
                retention_override: self.retention.as_deref(),
                global_state_dir: &runtime.paths().global_dir.join("state"),
                clock: &clock,
            },
        )
    }

    fn finish(&self, report: GcReport) -> Result<(), OrbitError> {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .map_err(|error| OrbitError::Execution(error.to_string()))?
            );
        } else {
            print_human_report(&report);
        }
        if report.has_errors() {
            return Err(OrbitError::Execution(format!(
                "garbage collection completed with {:?} outcome",
                report.outcome
            )));
        }
        Ok(())
    }
}

/// Task GC runs against the active workspace runtime, so its scope must be the
/// current workspace. Reject `--global` and any `--workspace` that resolves to
/// a different registered workspace (cross-workspace collection is reserved for
/// a future aggregate operator surface per the GC design contract).
fn ensure_current_workspace_scope(
    scope: GcScope,
    runtime: &OrbitRuntime,
) -> Result<GcScope, OrbitError> {
    match &scope {
        GcScope::Global { .. } => Err(OrbitError::InvalidInput(
            "task GC is workspace-scoped; drop --global and run it against a workspace".to_string(),
        )),
        GcScope::Workspace { root, .. } => {
            let current = &runtime.paths().orbit_dir;
            if paths_identical(root, current) {
                Ok(scope)
            } else {
                Err(OrbitError::InvalidInput(format!(
                    "task GC only collects the active workspace ({}); cross-workspace selection is not supported in v1",
                    current.display()
                )))
            }
        }
    }
}

fn paths_identical(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn resolve_workspace_scope(
    selector: Option<&str>,
    runtime: &OrbitRuntime,
) -> Result<GcScope, OrbitError> {
    let Some(selector) = selector else {
        return Ok(GcScope::Workspace {
            workspace_id: None,
            root: runtime.paths().orbit_dir.clone(),
        });
    };
    let registry_path =
        orbit_core::workspace_registry::registry_path_for(&runtime.paths().global_dir);
    let registry = orbit_core::workspace_registry::load_registry_from(&registry_path)?;
    let selector_path = PathBuf::from(selector);
    if selector_path.is_absolute() {
        let selected = selector_path.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!("cannot resolve workspace `{selector}`: {error}"))
        })?;
        let workspace = registry
            .workspaces
            .iter()
            .find(|workspace| {
                workspace
                    .root
                    .canonicalize()
                    .is_ok_and(|root| root == selected)
            })
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!("workspace path `{selector}` is not registered"))
            })?;
        return Ok(GcScope::Workspace {
            workspace_id: Some(workspace.id.clone()),
            root: workspace.orbit_dir.clone(),
        });
    }
    let workspace = orbit_core::workspace_registry::find_workspace(&registry, selector)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!("workspace `{selector}` is not registered"))
        })?;
    Ok(GcScope::Workspace {
        workspace_id: Some(workspace.id.clone()),
        root: workspace.orbit_dir.clone(),
    })
}

pub(crate) fn print_human_report(report: &orbit_core::command::gc::GcReport) {
    println!(
        "GC {:?} {} ({:?})",
        report.mode, report.plan_id, report.outcome
    );
    for target in &report.targets {
        println!(
            "{}: scanned {}, eligible {}, reclaimed {}; bytes scanned {}, eligible {}, reclaimed {}",
            target.target,
            target.counts.scanned,
            target.counts.eligible,
            target.counts.reclaimed,
            target.bytes.scanned,
            target.bytes.eligible,
            target.bytes.reclaimed,
        );
        for item in &target.items {
            println!(
                "  item {} [{:?}]: {} ({} bytes)",
                item.id,
                item.status,
                item.action,
                item.bytes
                    .map_or_else(|| "unknown".to_string(), |bytes| bytes.to_string())
            );
        }
        for skip in &target.skipped {
            println!("  skipped {} [{}]: {}", skip.id, skip.code, skip.reason);
        }
        for error in &target.errors {
            println!(
                "  error {} [{}:{}]: {}",
                error.id, error.phase, error.code, error.message
            );
        }
    }
    if let Some(path) = &report.manifest_path {
        println!("deletion manifest: {}", path.display());
    }
}

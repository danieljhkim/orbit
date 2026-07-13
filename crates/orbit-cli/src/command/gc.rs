use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use orbit_core::command::gc::{
    EmptyGcCollector, GcCollector, GcReport, GcRequest, GcScope, GcTarget, SystemGcClock,
    WorktreeGcCollector, WorktreeGcPolicy, execute_gc,
};
use orbit_core::command::gc_logs::LogsGcCollector;
use orbit_core::command::skill_gc::SkillsGcCollector;
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

    /// Override successful/cancelled worktree retention in days
    #[arg(long, value_name = "DAYS")]
    pub success_retention_days: Option<u64>,

    /// Override failed/timeout/interrupted worktree retention in days
    #[arg(long, value_name = "DAYS")]
    pub failure_retention_days: Option<u64>,

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
        if target != GcTarget::Worktrees
            && (self.success_retention_days.is_some() || self.failure_retention_days.is_some())
        {
            return Err(OrbitError::InvalidInput(
                "worktree retention overrides require the `worktrees` target".to_string(),
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
        let selected_runtime =
            if target == GcTarget::Worktrees && scope.root() != runtime.paths().orbit_dir {
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

fn print_human_report(report: &orbit_core::command::gc::GcReport) {
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

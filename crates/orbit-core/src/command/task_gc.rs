//! Age-based archival collector for terminal tasks (`orbit gc tasks`).
//!
//! This is the domain collector for [`GcTarget::Tasks`]; it plugs into the
//! shared plan/apply framework in [`super::gc`]. Per the GC design contract
//! (`docs/design/gc/2_design.md` §3.7), v1 task collection is **reversible
//! archival, never physical bundle deletion**:
//!
//! - Only configured terminal statuses are age-selected (`done`, and — when
//!   [`TaskGcCollector::include_rejected`] is set — `rejected`).
//! - Eligibility is measured from the persisted transition **into** the
//!   terminal status ([`terminal_transition_at`]), never `created_at`,
//!   `updated_at`, or filesystem mtime.
//! - Active/non-terminal states, a keep tag, open review threads, and
//!   unresolved lifecycle coupling (an active task that still depends on this
//!   one) are protected and retained with a reason.
//! - Apply delegates to the ordinary [`OrbitRuntime::archive_task`] lifecycle
//!   mutation, so history, audit, projections, relations, and search indexes
//!   stay consistent and restoration (`orbit task update <id> --status
//!   backlog`) remains supported.

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{OrbitError, ReviewThreadStatus, Task, TaskHistoryEntry, TaskStatus};

use crate::OrbitRuntime;

use super::gc::{
    GcCandidate, GcCollector, GcContext, GcMutation, GcPlan, GcRevalidation, GcSkip, GcTarget,
};

/// Tag that exempts a terminal task from age-based archival.
pub const GC_KEEP_TAG: &str = "gc-keep";

/// Built-in retention: terminal tasks younger than this are never age-selected.
const DEFAULT_TASK_RETENTION_DAYS: i64 = 90;

/// Age-based archival collector for terminal tasks.
pub struct TaskGcCollector<'a> {
    runtime: &'a OrbitRuntime,
    default_retention: Duration,
    include_rejected: bool,
    keep_tag: String,
}

impl<'a> TaskGcCollector<'a> {
    /// Builds a collector with the built-in retention, `done`-only terminal
    /// selection, and the standard [`GC_KEEP_TAG`] exemption.
    pub fn new(runtime: &'a OrbitRuntime) -> Self {
        Self {
            runtime,
            default_retention: Duration::days(DEFAULT_TASK_RETENTION_DAYS),
            include_rejected: false,
            keep_tag: GC_KEEP_TAG.to_string(),
        }
    }

    /// Also age-select `rejected` tasks (the optional terminal status).
    pub fn include_rejected(mut self, include: bool) -> Self {
        self.include_rejected = include;
        self
    }

    fn is_terminal(&self, status: TaskStatus) -> bool {
        status == TaskStatus::Done || (self.include_rejected && status == TaskStatus::Rejected)
    }

    fn effective_retention(&self, context: &GcContext<'_>) -> Result<Duration, OrbitError> {
        match context.retention_override {
            Some(raw) => parse_retention(raw),
            None => Ok(self.default_retention),
        }
    }

    /// Classifies a single task against the retention clock and the protection
    /// invariants. `all_tasks` is the workspace snapshot, used to detect an
    /// active task that still depends on this one.
    fn classify(
        &self,
        task: &Task,
        all_tasks: &[Task],
        now: DateTime<Utc>,
        retention: Duration,
    ) -> Result<Eligibility, OrbitError> {
        if !self.is_terminal(task.status) {
            return Ok(Eligibility::NotTerminal);
        }

        let history = self.runtime.get_task_history(&task.id)?;
        let Some(terminal_at) = terminal_transition_at(&history, task.status) else {
            // A terminal task without a recorded terminal transition (legacy or
            // hand-edited state) is ambiguous — retain it rather than age it by
            // `updated_at`.
            return Ok(Eligibility::Preserved {
                code: "missing_terminal_transition",
                reason: format!(
                    "task '{}' has no recorded transition into '{}'",
                    task.id, task.status
                ),
            });
        };

        // "older than the retention age": strict, so a task exactly at the
        // boundary is retained for one more clock tick.
        if terminal_at >= now - retention {
            return Ok(Eligibility::TooYoung);
        }

        if task
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(&self.keep_tag))
        {
            return Ok(Eligibility::Preserved {
                code: "keep_tag",
                reason: format!("task carries the '{}' keep tag", self.keep_tag),
            });
        }

        let open_threads = self
            .runtime
            .get_task_review_threads(&task.id)?
            .into_iter()
            .filter(|thread| thread.status == ReviewThreadStatus::Open)
            .count();
        if open_threads > 0 {
            return Ok(Eligibility::Preserved {
                code: "open_review_threads",
                reason: format!("{open_threads} open review thread(s) still reference this task"),
            });
        }

        if let Some(dependent) = active_dependent(task, all_tasks) {
            return Ok(Eligibility::Preserved {
                code: "active_dependency",
                reason: format!("active task '{dependent}' still couples to this task"),
            });
        }

        Ok(Eligibility::Candidate {
            terminal_at,
            status: task.status,
        })
    }
}

/// Result of classifying one task.
enum Eligibility {
    /// Old enough and unprotected: archive it.
    Candidate {
        terminal_at: DateTime<Utc>,
        status: TaskStatus,
    },
    /// Terminal but still within the retention window.
    TooYoung,
    /// Not in a selectable terminal status.
    NotTerminal,
    /// Old enough but held back by a protection invariant.
    Preserved { code: &'static str, reason: String },
}

impl GcCollector for TaskGcCollector<'_> {
    fn target(&self) -> GcTarget {
        GcTarget::Tasks
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let retention = self.effective_retention(context)?;
        let now = context.clock.now();
        let tasks = self.runtime.list_tasks()?;

        let mut candidates = Vec::new();
        let mut skipped = Vec::new();
        for task in &tasks {
            match self.classify(task, &tasks, now, retention)? {
                Eligibility::Candidate {
                    terminal_at,
                    status,
                } => candidates.push(GcCandidate {
                    id: task.id.clone(),
                    action: "archive".to_string(),
                    // Task archival is a lifecycle mutation, not a filesystem
                    // deletion, so there is no path to containment-check and no
                    // byte estimate to report.
                    path: None,
                    bytes: None,
                    ownership_evidence: format!(
                        "workspace task '{}' in terminal status '{status}'",
                        task.id
                    ),
                    retention_evidence: format!(
                        "transition into '{status}' at {} is older than retention {}",
                        terminal_at.to_rfc3339(),
                        humanize_duration(retention)
                    ),
                    expected_state: status.cli_name().to_string(),
                    allow_owned_symlink: false,
                }),
                Eligibility::Preserved { code, reason } => skipped.push(GcSkip {
                    id: task.id.clone(),
                    code: code.to_string(),
                    reason,
                }),
                Eligibility::TooYoung | Eligibility::NotTerminal => {}
            }
        }

        Ok(GcPlan {
            target: GcTarget::Tasks,
            config_source: "builtin".to_string(),
            scanned: tasks.len() as u64,
            // Archival reclaims no disk in v1; leave byte accounting unknown.
            scanned_bytes: None,
            candidates,
            skipped,
            errors: Vec::new(),
        })
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let retention = self.effective_retention(context)?;
        let now = context.clock.now();
        let task = match self.runtime.get_task(&candidate.id) {
            Ok(task) => task,
            Err(OrbitError::NotFound { .. }) => {
                return Ok(GcRevalidation::Skip {
                    code: "state_changed".to_string(),
                    reason: format!("task '{}' no longer exists", candidate.id),
                });
            }
            Err(error) => return Err(error),
        };
        let tasks = self.runtime.list_tasks()?;
        match self.classify(&task, &tasks, now, retention)? {
            Eligibility::Candidate { .. } => Ok(GcRevalidation::Ready),
            Eligibility::Preserved { code, reason } => Ok(GcRevalidation::Skip {
                code: code.to_string(),
                reason,
            }),
            Eligibility::TooYoung => Ok(GcRevalidation::Skip {
                code: "within_retention".to_string(),
                reason: format!("task '{}' is now within the retention window", candidate.id),
            }),
            Eligibility::NotTerminal => Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: format!("task '{}' is now in status '{}'", candidate.id, task.status),
            }),
        }
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        self.runtime.archive_task(&candidate.id)?;
        Ok(GcMutation {
            reclaimed_bytes: None,
        })
    }
}

/// Returns the timestamp of the most recent history transition **into**
/// `status`, or `None` when no such transition is recorded.
fn terminal_transition_at(
    history: &[TaskHistoryEntry],
    status: TaskStatus,
) -> Option<DateTime<Utc>> {
    history
        .iter()
        .filter(|entry| entry.to_status == Some(status))
        .map(|entry| entry.at)
        .max()
}

/// Returns the id of an active (non-closed) task that still couples to `task`
/// through a dependency or parent relation, if any.
fn active_dependent(task: &Task, all_tasks: &[Task]) -> Option<String> {
    all_tasks
        .iter()
        .filter(|other| other.id != task.id && !is_closed(other.status))
        .find(|other| {
            other.dependencies().iter().any(|dep| dep == &task.id)
                || other.parent_id() == Some(task.id.as_str())
        })
        .map(|other| other.id.clone())
}

/// A task whose lifecycle has settled: it neither blocks work nor will change
/// again on its own, so it cannot represent unresolved coupling.
fn is_closed(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Done | TaskStatus::Rejected | TaskStatus::Archived
    )
}

/// Parses a retention string like `90d`, `12w`, `24h`, `30m`, or `3600s` into a
/// [`Duration`]. Mirrors the CLI `--since`/duration grammar (`s/m/h/d/w`).
fn parse_retention(raw: &str) -> Result<Duration, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(
            "retention duration must not be empty".to_string(),
        ));
    }
    let split_at = value
        .find(|c: char| c.is_alphabetic())
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid retention duration: {raw}")))?;
    let (number, unit) = value.split_at(split_at);
    let number: i64 = number.parse().map_err(|_| {
        OrbitError::InvalidInput(format!("invalid retention duration number: {raw}"))
    })?;
    if number < 0 {
        return Err(OrbitError::InvalidInput(format!(
            "retention duration must not be negative: {raw}"
        )));
    }
    let duration = match unit {
        "s" => Duration::try_seconds(number),
        "m" => Duration::try_minutes(number),
        "h" => Duration::try_hours(number),
        "d" => Duration::try_days(number),
        "w" => Duration::try_weeks(number),
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid retention unit: {other} (expected s/m/h/d/w)"
            )));
        }
    };
    duration.ok_or_else(|| {
        OrbitError::InvalidInput(format!("retention '{raw}' is too large to represent"))
    })
}

/// Renders a whole-unit retention window back into the compact grammar for
/// human-readable evidence strings.
fn humanize_duration(duration: Duration) -> String {
    let seconds = duration.num_seconds();
    if seconds != 0 && seconds % 604_800 == 0 {
        format!("{}w", seconds / 604_800)
    } else if seconds != 0 && seconds % 86_400 == 0 {
        format!("{}d", seconds / 86_400)
    } else if seconds != 0 && seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{seconds}s")
    }
}

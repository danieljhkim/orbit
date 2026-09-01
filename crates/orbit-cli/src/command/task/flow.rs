//! `orbit task flow` — filed-versus-closed rates for a task population.
//!
//! A non-blocking review model files findings instead of blocking merges, so
//! the question that decides whether it is working cannot be answered from a
//! snapshot: are findings closing as fast as they are filed? An open count
//! alone cannot tell the two apart — a steady forty open tasks is healthy when
//! forty close a week and terminal when none do. This bucketizes both rates
//! over the same windows so the trend is visible.
//!
//! Closure time is approximated by `updated_at` on a task that has reached a
//! terminal status. `Done` is terminal, so nothing moves the timestamp
//! afterwards, but a task edited shortly before it closed reports that edit's
//! time. The error is bounded by the gap between a task's last edit and its
//! close, and it does not accumulate across buckets.

use chrono::{DateTime, Duration, Utc};
use clap::{ArgAction, Args};
use orbit_core::{OrbitError, OrbitRuntime, Task, TaskStatus, TaskType};
use serde_json::json;

use crate::command::{Block, CommandOut, Execute, Payload};
use crate::output::table::{Column, Table};
use crate::parse::parse_duration_seconds;

/// Default bucket width. A week matches how the review sweeps are scheduled
/// and how a human reads backlog movement.
const DEFAULT_WINDOW: &str = "7d";
/// Default number of buckets. Six weeks is long enough for a trend to separate
/// itself from one noisy week.
const DEFAULT_BUCKETS: usize = 6;

#[derive(Args)]
#[command(
    about = "Show filed-vs-closed rates over time (is the backlog draining?)",
    after_help = "Examples:\n  orbit task flow\n  orbit task flow --tag code-review\n  orbit task flow --tag security-review --window 14d --buckets 4\n  orbit task flow --type bug --json\n\n\
                  FILED counts tasks created in the window. CLOSED counts tasks that reached\n\
                  `done`; DROPPED counts `rejected` and `archived`, which clear the backlog\n\
                  without fixing anything — a drain driven by DROPPED is not the same result\n\
                  as one driven by CLOSED. NET is FILED minus both. OPEN AT END is how much\n\
                  of the population was still open when that window closed.\n\n\
                  The verdict compares total inflow against total outflow across every\n\
                  window: draining, flat, or growing.\n\n\
                  Closure time is approximated by a terminal task's last-updated timestamp;\n\
                  a task edited shortly before closing reports that edit's time."
)]
pub struct TaskFlowArgs {
    /// Filter by tag. Repeat for AND semantics, matching `orbit task list`.
    #[arg(long = "tag", action = ArgAction::Append, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Filter by task type (feature, bug, refactor, chore)
    #[arg(long = "type", value_enum)]
    pub task_type: Option<TaskType>,
    /// Width of each bucket as a duration (s/m/h/d/w). Default 7d.
    #[arg(long, default_value = DEFAULT_WINDOW)]
    pub window: String,
    /// Number of buckets to report, most recent last. Default 6.
    #[arg(long, default_value_t = DEFAULT_BUCKETS, value_parser = parse_buckets)]
    pub buckets: usize,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

/// The three task fields the report reads. Reducing to them keeps the
/// arithmetic testable without constructing whole tasks, and makes it obvious
/// that nothing else influences the numbers.
#[derive(Clone, Copy)]
pub(crate) struct FlowPoint {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal: Option<TerminalKind>,
}

/// A task leaves the live backlog on `done`, `rejected`, or `archived`. The
/// first is delivery; the other two clear the queue without it, which is why
/// they are counted apart rather than summed into one "closed" number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TerminalKind {
    Closed,
    Dropped,
}

impl TerminalKind {
    fn of(status: TaskStatus) -> Option<Self> {
        match status {
            TaskStatus::Done => Some(Self::Closed),
            TaskStatus::Rejected | TaskStatus::Archived => Some(Self::Dropped),
            _ => None,
        }
    }
}

impl FlowPoint {
    pub(crate) fn from_task(task: &Task) -> Self {
        Self {
            created_at: task.created_at,
            updated_at: task.updated_at,
            terminal: TerminalKind::of(task.status),
        }
    }

    /// Whether the task was still open at `instant`: created by then, and
    /// either never closed or closed afterwards.
    fn open_at(&self, instant: DateTime<Utc>) -> bool {
        self.created_at <= instant && (self.terminal.is_none() || self.updated_at > instant)
    }
}

/// One bucket's inflow and outflow, plus the population still open at its end.
pub(crate) struct Bucket {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub filed: usize,
    pub closed: usize,
    pub dropped: usize,
    pub open_at_end: usize,
}

impl Bucket {
    pub(crate) fn net(&self) -> i64 {
        self.filed as i64 - self.closed as i64 - self.dropped as i64
    }
}

pub(crate) struct FlowReport {
    pub buckets: Vec<Bucket>,
    pub filed: usize,
    pub closed: usize,
    pub dropped: usize,
    pub open_now: usize,
}

impl FlowReport {
    pub(crate) fn net(&self) -> i64 {
        self.filed as i64 - self.closed as i64 - self.dropped as i64
    }

    /// Inflow against outflow across the whole reported span, worded as the
    /// answer to the question the command exists to ask.
    ///
    /// An empty population reports that it has no data rather than "flat":
    /// zero equals zero is arithmetically a match and substantively nothing,
    /// and a caller who filtered down to no tasks must not read it as health.
    pub(crate) fn verdict(&self) -> &'static str {
        if self.filed == 0 && self.closed == 0 && self.dropped == 0 && self.open_now == 0 {
            return "no data — no tasks matched in the reported span";
        }
        match self.net() {
            net if net > 0 => "growing — filed faster than cleared",
            0 => "flat — inflow and outflow match",
            _ => "draining — cleared faster than filed",
        }
    }
}

/// Bucket the population into `count` windows of `width`, oldest first, ending
/// at `now`. `now` is a parameter rather than read inside so the arithmetic is
/// deterministic under test.
pub(crate) fn compute_flow(
    points: &[FlowPoint],
    now: DateTime<Utc>,
    width: Duration,
    count: usize,
) -> FlowReport {
    let mut buckets: Vec<Bucket> = (0..count)
        .rev()
        .map(|index| {
            let end = now - width * (index as i32);
            Bucket {
                start: end - width,
                end,
                filed: 0,
                closed: 0,
                dropped: 0,
                open_at_end: 0,
            }
        })
        .collect();

    for bucket in &mut buckets {
        for point in points {
            if point.created_at >= bucket.start && point.created_at < bucket.end {
                bucket.filed += 1;
            }
            if let Some(kind) = point.terminal
                && point.updated_at >= bucket.start
                && point.updated_at < bucket.end
            {
                match kind {
                    TerminalKind::Closed => bucket.closed += 1,
                    TerminalKind::Dropped => bucket.dropped += 1,
                }
            }
            if point.open_at(bucket.end) {
                bucket.open_at_end += 1;
            }
        }
    }

    FlowReport {
        filed: buckets.iter().map(|bucket| bucket.filed).sum(),
        closed: buckets.iter().map(|bucket| bucket.closed).sum(),
        dropped: buckets.iter().map(|bucket| bucket.dropped).sum(),
        open_now: points.iter().filter(|point| point.open_at(now)).count(),
        buckets,
    }
}

impl Execute for TaskFlowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let width = bucket_width(&self.window)?;
        let points: Vec<FlowPoint> = runtime
            .list_tasks_by_tags(&self.tags)?
            .iter()
            .filter(|task| self.task_type.is_none_or(|kind| task.task_type == kind))
            .map(FlowPoint::from_task)
            .collect();

        let report = compute_flow(&points, Utc::now(), width, self.buckets);

        let mut table = Table::new(vec![
            Column::new("WINDOW").fixed(),
            Column::new("FILED").number(),
            Column::new("CLOSED").number(),
            Column::new("DROPPED").number(),
            Column::new("NET").number(),
            Column::new("OPEN AT END").number(),
        ])
        .keep_all_columns()
        .empty_message("no windows to report");

        for bucket in &report.buckets {
            table.add_row(vec![
                bucket.start.format("%Y-%m-%d").to_string(),
                bucket.filed.to_string(),
                bucket.closed.to_string(),
                bucket.dropped.to_string(),
                format_net(bucket.net()),
                bucket.open_at_end.to_string(),
            ]);
        }

        let doc = json!({
            "window": self.window,
            "buckets": report
                .buckets
                .iter()
                .map(|bucket| json!({
                    "start": bucket.start.to_rfc3339(),
                    "end": bucket.end.to_rfc3339(),
                    "filed": bucket.filed,
                    "closed": bucket.closed,
                    "dropped": bucket.dropped,
                    "net": bucket.net(),
                    "open_at_end": bucket.open_at_end,
                }))
                .collect::<Vec<_>>(),
            "totals": {
                "filed": report.filed,
                "closed": report.closed,
                "dropped": report.dropped,
                "net": report.net(),
                "open_now": report.open_now,
            },
            "verdict": report.verdict(),
        });

        let summary = format!(
            "{} filed, {} closed, {} dropped over {} × {} — net {}, {} open now: {}",
            report.filed,
            report.closed,
            report.dropped,
            self.buckets,
            self.window,
            format_net(report.net()),
            report.open_now,
            report.verdict(),
        );
        Ok(Payload::blocks(doc, vec![Block::table(table), Block::text(summary)]).into())
    }
}

/// Signed rendering, so a negative net — the healthy direction — is
/// unmistakable next to an unsigned count.
pub(crate) fn format_net(net: i64) -> String {
    if net > 0 {
        format!("+{net}")
    } else {
        net.to_string()
    }
}

/// Bucket width as a `chrono::Duration`, rejecting a zero width that would
/// produce empty windows.
fn bucket_width(raw: &str) -> Result<Duration, OrbitError> {
    let seconds = parse_duration_seconds(raw)?;
    if seconds == 0 {
        return Err(OrbitError::InvalidInput(
            "window must be longer than zero".to_string(),
        ));
    }
    let seconds = i64::try_from(seconds)
        .map_err(|_| OrbitError::InvalidInput(format!("window '{raw}' is too large")))?;
    Duration::try_seconds(seconds)
        .ok_or_else(|| OrbitError::InvalidInput(format!("window '{raw}' is too large")))
}

/// Reject a zero bucket count, which would report nothing at all.
fn parse_buckets(raw: &str) -> Result<usize, String> {
    let value: usize = raw.parse().map_err(|_| {
        format!("`{raw}` is not a valid bucket count (expected a positive integer)")
    })?;
    if value == 0 {
        return Err("buckets must be at least 1".to_string());
    }
    Ok(value)
}

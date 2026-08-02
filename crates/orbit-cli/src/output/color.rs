use colored::Colorize;
use comfy_table::{Attribute, Cell, Color as TableColor};

/// The closed set of semantic roles a domain value can carry.
/// See `docs/design/terminal-interface/specs/color-and-styling.md` (ADR-0308).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    Ok,
    Warn,
    Error,
    Active,
    Muted,
    Neutral,
}

/// The domain vocabularies this crate colors. Distinguishes values that read
/// the same across domains (e.g. `"active"` as a job state vs. `"archived"`
/// as a task status) so each gets the role its own vocabulary intends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Domain {
    TaskStatus,
    Priority,
    TaskType,
    JobState,
    DoctorStatus,
}

/// The single value-to-role mapping covering every domain vocabulary in the
/// crate. Unmapped values resolve to `Role::Neutral` — never a panic, never
/// an arbitrary color. Adding a status is a one-line edit here.
pub(crate) fn role_for(domain: Domain, value: &str) -> Role {
    use Domain::*;
    use Role::*;
    match (domain, value) {
        (TaskStatus, "proposed") => Warn,
        (TaskStatus, "in-progress") => Active,
        (TaskStatus, "done") => Ok,
        (TaskStatus, "rejected" | "blocked") => Error,
        (TaskStatus, "archived") => Muted,
        // "review" and "backlog" have no dedicated role; see color.rs's
        // module-level completion notes for the resulting collapses.
        (Priority, "high") => Error,
        (Priority, "medium") => Warn,
        (Priority, "low") => Muted,

        (JobState, "success" | "active") => Ok,
        (JobState, "failed" | "error" | "disabled") => Error,
        (JobState, "running") => Active,
        (JobState, "pending") => Warn,

        (DoctorStatus, "ok") => Ok,
        (DoctorStatus, "warning") => Warn,
        (DoctorStatus, "ERROR" | "error") => Error,

        // TaskType carries no role today; every value renders neutral.
        _ => Neutral,
    }
}

impl Role {
    pub(crate) fn table_color(self) -> Option<TableColor> {
        match self {
            Role::Ok => Some(TableColor::Green),
            Role::Warn => Some(TableColor::Yellow),
            Role::Error => Some(TableColor::Red),
            Role::Active => Some(TableColor::Cyan),
            Role::Muted | Role::Neutral => None,
        }
    }

    pub(crate) fn line_color(self) -> Option<colored::Color> {
        match self {
            Role::Ok => Some(colored::Color::Green),
            Role::Warn => Some(colored::Color::Yellow),
            Role::Error => Some(colored::Color::Red),
            Role::Active => Some(colored::Color::Cyan),
            Role::Muted | Role::Neutral => None,
        }
    }

    pub(crate) fn is_dim(self) -> bool {
        matches!(self, Role::Muted)
    }
}

fn cell_for(value: &str, role: Role) -> Cell {
    let cell = Cell::new(value);
    let cell = match role.table_color() {
        Some(color) => cell.fg(color),
        None => cell,
    };
    if role.is_dim() {
        cell.add_attribute(Attribute::Dim)
    } else {
        cell
    }
}

fn string_for(value: &str, role: Role) -> String {
    let styled = match role.line_color() {
        Some(color) => value.color(color),
        None => value.normal(),
    };
    let styled = if role.is_dim() {
        styled.dimmed()
    } else {
        styled
    };
    styled.to_string()
}

pub fn status_color_cell(status: &str) -> Cell {
    cell_for(status, role_for(Domain::TaskStatus, status))
}

pub fn priority_color_cell(priority: &str) -> Cell {
    cell_for(priority, role_for(Domain::Priority, priority))
}

pub fn task_type_color_cell(task_type: &str) -> Cell {
    cell_for(task_type, role_for(Domain::TaskType, task_type))
}

pub fn job_state_color_cell(state: &str) -> Cell {
    cell_for(state, role_for(Domain::JobState, state))
}

pub fn doctor_status_color_cell(status: &str) -> Cell {
    cell_for(status, role_for(Domain::DoctorStatus, status))
}

pub fn status_color(status: &str) -> String {
    string_for(status, role_for(Domain::TaskStatus, status))
}

pub fn priority_color(priority: &str) -> String {
    string_for(priority, role_for(Domain::Priority, priority))
}

pub fn job_state_color(state: &str) -> String {
    string_for(state, role_for(Domain::JobState, state))
}

pub fn bold(text: &str) -> String {
    text.bold().to_string()
}

pub fn dimmed(text: &str) -> String {
    text.dimmed().to_string()
}

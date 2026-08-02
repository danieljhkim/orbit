use comfy_table::{Attribute, Cell};

use super::super::color::{
    Domain, Role, doctor_status_color_cell, job_state_color, job_state_color_cell, priority_color,
    priority_color_cell, role_for, status_color, status_color_cell, task_type_color_cell,
};

fn expected_cell(value: &str, role: Role) -> Cell {
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

fn expected_string(value: &str, role: Role) -> String {
    use colored::Colorize;
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

/// (domain, value, expected role). One entry per mapped value, plus a
/// representative unmapped value per domain.
const CASES: &[(Domain, &str, Role)] = &[
    (Domain::TaskStatus, "proposed", Role::Warn),
    (Domain::TaskStatus, "in-progress", Role::Active),
    (Domain::TaskStatus, "done", Role::Ok),
    (Domain::TaskStatus, "rejected", Role::Error),
    (Domain::TaskStatus, "blocked", Role::Error),
    (Domain::TaskStatus, "archived", Role::Muted),
    (Domain::TaskStatus, "review", Role::Neutral),
    (Domain::TaskStatus, "backlog", Role::Neutral),
    (Domain::TaskStatus, "some-future-status", Role::Neutral),
    (Domain::Priority, "high", Role::Error),
    (Domain::Priority, "medium", Role::Warn),
    (Domain::Priority, "low", Role::Muted),
    (Domain::Priority, "unspecified", Role::Neutral),
    (Domain::JobState, "success", Role::Ok),
    (Domain::JobState, "active", Role::Ok),
    (Domain::JobState, "failed", Role::Error),
    (Domain::JobState, "error", Role::Error),
    (Domain::JobState, "disabled", Role::Error),
    (Domain::JobState, "running", Role::Active),
    (Domain::JobState, "pending", Role::Warn),
    (Domain::JobState, "queued", Role::Neutral),
    (Domain::TaskType, "feature", Role::Neutral),
    (Domain::TaskType, "bug", Role::Neutral),
    (Domain::DoctorStatus, "ok", Role::Ok),
    (Domain::DoctorStatus, "warning", Role::Warn),
    (Domain::DoctorStatus, "ERROR", Role::Error),
    (Domain::DoctorStatus, "error", Role::Error),
    (Domain::DoctorStatus, "unknown", Role::Neutral),
];

#[test]
fn mapping_matches_expected_role() {
    for (domain, value, role) in CASES {
        assert_eq!(
            role_for(*domain, value),
            *role,
            "{domain:?}/{value} should map to {role:?}"
        );
    }
}

#[test]
fn cell_and_string_forms_agree_for_every_mapped_value() {
    for (domain, value, role) in CASES {
        match domain {
            Domain::TaskStatus => {
                assert_eq!(status_color_cell(value), expected_cell(value, *role));
                assert_eq!(status_color(value), expected_string(value, *role));
            }
            Domain::Priority => {
                assert_eq!(priority_color_cell(value), expected_cell(value, *role));
                assert_eq!(priority_color(value), expected_string(value, *role));
            }
            Domain::JobState => {
                assert_eq!(job_state_color_cell(value), expected_cell(value, *role));
                assert_eq!(job_state_color(value), expected_string(value, *role));
            }
            Domain::TaskType => {
                assert_eq!(task_type_color_cell(value), expected_cell(value, *role));
            }
            Domain::DoctorStatus => {
                assert_eq!(doctor_status_color_cell(value), expected_cell(value, *role));
            }
        }
    }
}

#[test]
fn unmapped_value_is_neutral_and_does_not_panic() {
    assert_eq!(
        role_for(Domain::TaskStatus, "totally-unknown"),
        Role::Neutral
    );
    assert_eq!(
        status_color_cell("totally-unknown"),
        Cell::new("totally-unknown")
    );
    assert_eq!(status_color("totally-unknown"), "totally-unknown");
}

#[test]
fn backlog_is_no_longer_inconsistent_between_forms() {
    // Historically `status_color` mapped "backlog" explicitly (to no style)
    // while `status_color_cell` fell through the wildcard arm (also to no
    // style) -- same rendering, but two divergent code paths. Both now go
    // through the same table entry (the wildcard), which this asserts by
    // comparing them directly against the plain, unstyled form.
    assert_eq!(status_color_cell("backlog"), Cell::new("backlog"));
    assert_eq!(status_color("backlog"), "backlog");
}

use orbit_cmd::{WorkspaceDoctorResult, WorkspaceDoctorStatus};

use super::super::doctor::{doctor_row_json, human_detail};

#[test]
fn doctor_warning_renders_structured_and_human_remediation() {
    let row = WorkspaceDoctorResult {
        check_name: "task-reservations".to_string(),
        status: WorkspaceDoctorStatus::Warning,
        message: "reservation-123 is stale".to_string(),
        remediation: Some("Run `orbit doctor --fix-stale-task-locks`.".to_string()),
    };

    let json = doctor_row_json(&row);
    assert_eq!(
        json["remediation"],
        "Run `orbit doctor --fix-stale-task-locks`."
    );
    let human = human_detail(&row);
    assert!(human.contains("reservation-123 is stale"), "{human}");
    assert!(
        human.contains("Action: Run `orbit doctor --fix-stale-task-locks`."),
        "{human}"
    );
}

#[test]
fn healthy_doctor_row_has_null_remediation_and_no_action_line() {
    let row = WorkspaceDoctorResult {
        check_name: "task-reservations".to_string(),
        status: WorkspaceDoctorStatus::Ok,
        message: "none stale".to_string(),
        remediation: None,
    };

    assert!(doctor_row_json(&row)["remediation"].is_null());
    assert_eq!(human_detail(&row), "none stale");
}

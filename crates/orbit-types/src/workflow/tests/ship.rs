use chrono::Utc;

use crate::workflow::{ShipMode, resolved_ship_mode};
use crate::workspace::{Workspace, WorkspaceStatus};

fn workspace(ship_mode: Option<&str>) -> Workspace {
    Workspace {
        id: "ws_test".to_string(),
        name: "test".to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: ship_mode.map(str::to_string),
        base_branch: "main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn ship_mode_parses_the_persisted_values() {
    assert_eq!(ShipMode::parse("pr"), Ok(ShipMode::Pr));
    assert_eq!(ShipMode::parse("local"), Ok(ShipMode::Local));
    assert!(ShipMode::parse("unknown").is_err());
}

#[test]
fn workspace_ship_mode_defaults_fail_closed_to_pr() {
    assert_eq!(resolved_ship_mode(&workspace(None)), ShipMode::Pr);
    assert_eq!(
        resolved_ship_mode(&workspace(Some("unknown"))),
        ShipMode::Pr
    );
    assert_eq!(
        resolved_ship_mode(&workspace(Some("local"))),
        ShipMode::Local
    );
}

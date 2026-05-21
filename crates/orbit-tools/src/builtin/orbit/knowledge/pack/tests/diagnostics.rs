//! Tests for refresh diagnostics in pack output.

use serde_json::json;

use super::super::add_refresh_diagnostics;

#[test]
fn refresh_diagnostics_only_report_actual_auto_refresh_skips() {
    let mut refreshed_pack = json!({"entries": []});
    add_refresh_diagnostics(&mut refreshed_pack, false, None, false);
    assert!(refreshed_pack.get("diagnostics").is_none());

    let mut skipped_pack = json!({"entries": []});
    add_refresh_diagnostics(&mut skipped_pack, true, None, false);
    assert_eq!(
        skipped_pack["diagnostics"]["auto_refresh"]["status"],
        "skipped"
    );
}

//! Tests for refresh diagnostics in pack output.

use orbit_knowledge::KnowledgePackResult;

use super::super::add_refresh_diagnostics;

#[test]
fn refresh_diagnostics_only_report_actual_auto_refresh_skips() {
    let mut refreshed_pack = KnowledgePackResult::default();
    add_refresh_diagnostics(&mut refreshed_pack, false, None, false);
    assert!(refreshed_pack.diagnostics.is_none());

    let mut skipped_pack = KnowledgePackResult::default();
    add_refresh_diagnostics(&mut skipped_pack, true, None, false);
    assert_eq!(
        skipped_pack
            .diagnostics
            .and_then(|diagnostics| diagnostics.auto_refresh)
            .map(|diagnostic| diagnostic.status),
        Some("skipped".to_string())
    );
}

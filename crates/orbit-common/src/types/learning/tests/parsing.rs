use std::str::FromStr;

use super::super::*;

#[test]
fn learning_status_from_str_round_trips() {
    for status in [LearningStatus::Active, LearningStatus::Superseded] {
        let parsed: LearningStatus = status.as_str().parse().expect("parse");
        assert_eq!(parsed, status);
    }
    assert!(LearningStatus::from_str("nope").is_err());
}

#[test]
fn evidence_kind_from_str_covers_all_variants() {
    assert_eq!(
        EvidenceKind::from_str("task").expect("task"),
        EvidenceKind::Task
    );
    assert_eq!(
        EvidenceKind::from_str("commit").expect("commit"),
        EvidenceKind::Commit
    );
    assert_eq!(
        EvidenceKind::from_str("external").expect("external"),
        EvidenceKind::External
    );
    assert!(EvidenceKind::from_str("other").is_err());
}

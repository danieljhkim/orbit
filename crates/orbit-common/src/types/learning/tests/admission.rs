use chrono::{TimeZone, Utc};

use super::super::*;

#[test]
fn learning_injection_state_dedupes_without_consuming_hard_cap() {
    let caps = LearningInjectionCaps {
        per_call: 5,
        per_session_hard: 2,
    };
    let mut state = LearningInjectionState::new();

    assert!(state.try_admit("L1", caps));
    assert!(!state.try_admit("L1", caps));
    assert!(state.try_admit("L2", caps));
    assert!(!state.try_admit("L3", caps));
    assert_eq!(state.count, 2);
    assert_eq!(state.emitted_ids.len(), 2);
}

#[test]
fn admit_reminders_enforces_per_call_cap() {
    let caps = LearningInjectionCaps {
        per_call: 2,
        per_session_hard: 20,
    };
    let mut state = LearningInjectionState::new();
    let reminders: Vec<_> = (0..4)
        .map(|idx| LearningReminder {
            id: format!("L{idx}"),
            summary: format!("summary {idx}"),
            comments: Vec::new(),
        })
        .collect();

    let admitted = state.admit_reminders(&reminders, caps);

    assert_eq!(
        admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["L0", "L1"]
    );
    assert_eq!(state.count, 2);
}

#[test]
fn decayed_vote_score_halves_each_half_life() {
    let now = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
    let recent = now;
    let old = now - chrono::Duration::days(180);

    let recent_weight = decayed_vote_score(&[recent], now, 180.0);
    let old_weight = decayed_vote_score(&[old], now, 180.0);

    let ratio = recent_weight / old_weight;
    assert!(
        (ratio - 2.0).abs() < 1e-6,
        "expected 2:1 ratio, got {ratio}"
    );
}

#[test]
fn decayed_vote_score_zero_half_life_returns_raw_count() {
    let now = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
    let votes = [
        now - chrono::Duration::days(30),
        now - chrono::Duration::days(730),
        now - chrono::Duration::days(1460),
    ];

    assert_eq!(decayed_vote_score(&votes, now, 0.0), 3.0);
}

use chrono::{TimeZone, Utc};

use super::super::*;

#[test]
fn render_reminder_block_returns_empty_for_no_reminders() {
    assert_eq!(render_reminder_block(&[]), "");
    assert_eq!(prepend_reminder_block("baseline", &[]), "baseline");
}

#[test]
fn render_reminder_block_matches_design_shape() {
    let block = render_reminder_block(&[LearningReminder {
        id: "L-0001".to_string(),
        summary: "Verify output equivalence before freezing a result.".to_string(),
        comments: Vec::new(),
    }]);

    assert_eq!(
        block,
        "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [L-0001] Verify output equivalence before freezing a result.\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>"
    );
}

#[test]
fn render_reminder_block_renders_comments_under_learning() {
    let ts = Utc.with_ymd_and_hms(2026, 5, 17, 0, 0, 0).unwrap();
    let block = render_reminder_block(&[LearningReminder {
        id: "L-0001".to_string(),
        summary: "Remember the important thing.".to_string(),
        comments: vec![LearningComment {
            id: "C20260517-1".to_string(),
            learning_id: "L-0001".to_string(),
            body: "Use the narrow helper.\nExtra detail stays hidden.".to_string(),
            author_model: "codex".to_string(),
            created_at: ts,
        }],
    }]);

    assert!(block.contains("- [L-0001] Remember the important thing.\n"));
    assert!(block.contains("  - [C20260517-1] Use the narrow helper.\n"));
}

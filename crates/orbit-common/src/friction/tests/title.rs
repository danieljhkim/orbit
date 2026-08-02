//! Title derivation reads body structure, not vocabulary.
//!
//! Every fixture below is synthetic. The generic section labels appear as
//! *inputs* the structural rules must survive — no rule anywhere consults a
//! list of them, and adding a new label spelling must not require a new case.

use crate::friction::title::{
    FRICTION_TITLE_MAX_CHARS, derive_title, effective_title, normalize_title,
};

/// A body with a single heading: the heading is the record's own title.
const SOLE_HEADING: &str = "\
## Task update refuses a record the sibling read just resolved

The read path scans every registered scope; the write path stops at the
default one.";

/// A structured report: sibling headings at the same level, so the first one
/// labels a section rather than the record.
const SECTIONED_REPORT: &str = "\
## What happened

The update verb rejected an id the show verb had just returned.

## Evidence

Two calls, one id, opposite outcomes.

## Why this is friction

The failure reads as a missing record rather than a routing miss.";

/// The same report shape written with bold lead-ins instead of headings.
const BOLD_LEAD_REPORT: &str = "\
**What happened.** The update verb rejected an id the show verb had just returned.

**Root cause.** Only the read path scans beyond the default scope.";

#[test]
fn a_sole_heading_is_the_records_own_title() {
    assert_eq!(
        derive_title(SOLE_HEADING).as_deref(),
        Some("Task update refuses a record the sibling read just resolved")
    );
}

#[test]
fn a_section_label_yields_the_prose_it_introduces() {
    // The label is skipped because peers follow it, not because of its words.
    assert_eq!(
        derive_title(SECTIONED_REPORT).as_deref(),
        Some("The update verb rejected an id the show verb had just returned.")
    );
}

#[test]
fn an_unfamiliar_section_label_is_skipped_on_the_same_evidence() {
    let body = SECTIONED_REPORT.replace("What happened", "Beobachtung");
    assert_eq!(
        derive_title(&body).as_deref(),
        Some("The update verb rejected an id the show verb had just returned.")
    );
}

#[test]
fn a_deeper_sibling_does_not_demote_a_leading_title() {
    let body = "# Dispatch drops a queued run\n\n## Evidence\n\nOne run, no worker.";
    assert_eq!(
        derive_title(body).as_deref(),
        Some("Dispatch drops a queued run")
    );
}

#[test]
fn a_bold_lead_in_yields_the_sentence_it_opens() {
    assert_eq!(
        derive_title(BOLD_LEAD_REPORT).as_deref(),
        Some("The update verb rejected an id the show verb had just returned.")
    );
}

#[test]
fn a_line_that_is_only_a_bold_run_is_the_title() {
    let body = "**Queued runs never reach a worker**\n\nThe dispatcher exits first.";
    assert_eq!(
        derive_title(body).as_deref(),
        Some("Queued runs never reach a worker")
    );
}

/// The unit is the paragraph, not the line. A hard-wrapped body would
/// otherwise be cut at whatever column its author's editor used.
#[test]
fn a_hard_wrapped_opening_reads_as_one_paragraph() {
    let body = "The update verb rejected an id\nthe show verb had just returned.\n\nMore detail.";

    assert_eq!(
        derive_title(body).as_deref(),
        Some("The update verb rejected an id the show verb had just returned.")
    );
}

#[test]
fn a_hard_wrapped_section_body_reads_as_one_paragraph() {
    let body = "## What happened\n\nThe update verb rejected an id\nthe show verb had just \
                returned.\n\n## Evidence\n\nTwo calls.";

    assert_eq!(
        derive_title(body).as_deref(),
        Some("The update verb rejected an id the show verb had just returned.")
    );
}

#[test]
fn a_headingless_paragraph_is_clamped_to_the_title_budget() {
    let body = format!("{} and then some more text.", "word ".repeat(60));
    let title = derive_title(&body).expect("derived title");

    assert!(
        title.chars().count() <= FRICTION_TITLE_MAX_CHARS,
        "{title} is {} characters",
        title.chars().count()
    );
    assert!(title.ends_with('…'), "{title}");
    assert!(!title.contains("  "), "{title}");
}

#[test]
fn a_clamped_title_cuts_at_a_word_boundary() {
    let body = "supercalifragilistic ".repeat(20);
    let title = derive_title(&body).expect("derived title");

    assert!(title.ends_with("supercalifragilistic…"), "{title}");
}

#[test]
fn a_short_body_is_not_clamped() {
    let title = derive_title("Dispatch drops a queued run.").expect("derived title");

    assert_eq!(title, "Dispatch drops a queued run.");
    assert!(!title.ends_with('…'));
}

#[test]
fn list_and_quote_markers_are_not_part_of_the_title() {
    assert_eq!(
        derive_title("- Dispatch drops a queued run").as_deref(),
        Some("Dispatch drops a queued run")
    );
    assert_eq!(
        derive_title("1. Dispatch drops a queued run").as_deref(),
        Some("Dispatch drops a queued run")
    );
    assert_eq!(
        derive_title("> Dispatch drops a queued run").as_deref(),
        Some("Dispatch drops a queued run")
    );
}

#[test]
fn a_body_that_is_only_headings_falls_back_to_the_first_one() {
    assert_eq!(
        derive_title("## What happened\n\n## Evidence").as_deref(),
        Some("What happened")
    );
}

#[test]
fn a_body_with_no_text_derives_nothing() {
    assert_eq!(derive_title("   \n\n\t\n"), None);
}

#[test]
fn a_stored_title_wins_over_the_body() {
    assert_eq!(
        effective_title(
            Some("Update ignores non-default scopes"),
            SECTIONED_REPORT,
            "F1"
        ),
        "Update ignores non-default scopes"
    );
}

#[test]
fn a_blank_stored_title_falls_through_to_derivation() {
    assert_eq!(
        effective_title(Some("   "), SOLE_HEADING, "F1"),
        "Task update refuses a record the sibling read just resolved"
    );
}

#[test]
fn a_record_with_neither_title_nor_text_falls_back_to_its_id() {
    assert_eq!(effective_title(None, "", "F0000-00-000"), "F0000-00-000");
}

#[test]
fn an_author_title_is_collapsed_to_one_line() {
    assert_eq!(
        normalize_title("  Update ignores\n  non-default scopes  ").expect("normalized"),
        "Update ignores non-default scopes"
    );
}

#[test]
fn a_blank_author_title_is_rejected() {
    let error = normalize_title(" \n ").expect_err("blank title");

    assert!(error.to_string().contains("must not be blank"), "{error}");
}

#[test]
fn an_author_title_past_the_budget_is_rejected() {
    let error = normalize_title(&"x".repeat(FRICTION_TITLE_MAX_CHARS + 1)).expect_err("long title");

    assert!(
        error
            .to_string()
            .contains(&FRICTION_TITLE_MAX_CHARS.to_string()),
        "{error}"
    );
}

#[test]
fn an_author_title_at_the_budget_is_accepted() {
    let title = "x".repeat(FRICTION_TITLE_MAX_CHARS);

    assert_eq!(normalize_title(&title).expect("boundary title"), title);
}

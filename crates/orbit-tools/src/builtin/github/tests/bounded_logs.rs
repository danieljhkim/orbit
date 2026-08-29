//! Bounding and redaction of runner-log excerpts, plus the checkout-evidence
//! scan that keeps the tested commit distinct from a run's reported head SHA.

use crate::builtin::github::{bound_log_text, scan_checkout_evidence};

#[test]
fn a_short_log_is_returned_whole_and_unmarked() {
    let bounded = bound_log_text("error: assertion failed\n", 1024);

    assert!(!bounded.truncated);
    assert_eq!(bounded.text, "error: assertion failed\n");
    assert_eq!(bounded.total_bytes, bounded.returned_bytes);
}

#[test]
fn an_oversized_log_is_capped_while_keeping_both_ends() {
    let raw = format!("HEAD-MARKER\n{}\nTAIL-MARKER", "x".repeat(200_000));

    let bounded = bound_log_text(&raw, 4_096);

    assert!(bounded.truncated);
    assert!(
        bounded.returned_bytes <= 4_096,
        "excerpt must respect the requested cap: {}",
        bounded.returned_bytes
    );
    assert_eq!(bounded.total_bytes, raw.len());
    assert!(bounded.text.contains("HEAD-MARKER"));
    assert!(bounded.text.contains("TAIL-MARKER"));
    assert!(bounded.text.contains("bytes omitted"));
}

#[test]
fn truncation_never_splits_a_multibyte_character() {
    let raw = "é".repeat(50_000);

    let bounded = bound_log_text(&raw, 1_001);

    assert!(bounded.truncated);
    // Reaching this point at all means both slices landed on char boundaries;
    // a mid-character split panics inside `bound_log_text`.
    assert!(bounded.text.contains('é'));
}

#[test]
fn a_credential_in_the_log_is_redacted_before_it_is_returned() {
    let raw = format!("remote: fatal: bad credentials ghp_{}\n", "a".repeat(36));

    let bounded = bound_log_text(&raw, 4_096);

    assert!(
        !bounded.text.contains("ghp_"),
        "token survived redaction: {}",
        bounded.text
    );
    assert!(bounded.text.contains("[REDACTED_SECRET]"));
}

#[test]
fn checkout_evidence_reports_the_tested_commit_not_the_event_sha() {
    let log = "\
setup\tRun actions/checkout@v4\t2026-01-01T00:00:00.0000000Z Syncing repository
setup\tRun actions/checkout@v4\t2026-01-01T00:00:01.0000000Z /usr/bin/git checkout --progress --force 1111111111111111111111111111111111111111
setup\tRun actions/checkout@v4\t2026-01-01T00:00:02.0000000Z HEAD is now at 1111111 chore: something
build\tRun tests\t2026-01-01T00:00:03.0000000Z error: build failed
";

    let evidence = scan_checkout_evidence(log, 40);

    assert!(
        evidence
            .commits
            .contains(&"1111111111111111111111111111111111111111".to_string()),
        "expected the full checked-out SHA: {:?}",
        evidence.commits
    );
    assert_eq!(evidence.lines.len(), 2);
    assert!(
        evidence
            .lines
            .iter()
            .any(|line| line.contains("HEAD is now at"))
    );
    assert!(
        evidence.lines.iter().all(|line| !line.contains('\t')),
        "the job and step columns are stripped: {:?}",
        evidence.lines
    );
}

/// The failure this scan is built to avoid. Every line of a checkout step is
/// labelled with the *action's* pinned SHA, and reporting that as the commit
/// under test would be evidence pointing at the wrong repository entirely.
#[test]
fn a_pinned_action_sha_is_never_reported_as_the_checked_out_commit() {
    let pin = "34e114876b0b11c390a56381ad16ebd13914f8d5";
    let tested = "5dbb8eff6f1ec88a24da618df1962d2c6b82ab6e";
    let log = format!(
        "\
macOS\tRun actions/checkout@{pin}\t2026-01-01T00:00:00.0000000Z ##[group]Checking out the ref
macOS\tRun actions/checkout@{pin}\t2026-01-01T00:00:01.0000000Z [command]/usr/bin/git checkout --progress --force -B trunk refs/remotes/origin/trunk
macOS\tRun actions/checkout@{pin}\t2026-01-01T00:00:02.0000000Z {tested}
"
    );

    let evidence = scan_checkout_evidence(&log, 40);

    assert!(
        !evidence.commits.contains(&pin.to_string()),
        "the action pin leaked into the checkout evidence: {:?}",
        evidence.commits
    );
    assert_eq!(evidence.commits, vec![tested.to_string()]);
}

/// A bare SHA is evidence only inside a checkout step. The same shape shows up
/// in ordinary build output, where it means nothing.
#[test]
fn a_bare_sha_outside_a_checkout_step_is_not_treated_as_evidence() {
    let log = "\
build\tRun tests\t2026-01-01T00:00:00.0000000Z 5dbb8eff6f1ec88a24da618df1962d2c6b82ab6e
";

    let evidence = scan_checkout_evidence(log, 40);

    assert!(evidence.commits.is_empty(), "{:?}", evidence.commits);
    assert!(evidence.lines.is_empty());
}

#[test]
fn checkout_evidence_lines_are_capped_and_redacted() {
    let line = format!(
        "setup\tcheckout\t2026-01-01T00:00:00.0000000Z HEAD is now at 2222222 token ghp_{}\n",
        "b".repeat(36)
    );
    let log = line.repeat(100);

    let evidence = scan_checkout_evidence(&log, 5);

    assert_eq!(evidence.lines.len(), 5);
    assert!(evidence.lines.iter().all(|line| !line.contains("ghp_")));
}

#[test]
fn a_log_without_checkout_evidence_yields_nothing_rather_than_a_guess() {
    let evidence = scan_checkout_evidence("build\tRun tests\terror: build failed\n", 40);

    assert!(evidence.commits.is_empty());
    assert!(evidence.lines.is_empty());
}

//! Fingerprint stability tests [ORB-10039]: the same failure must hash the
//! same across reruns (ANSI, whitespace, host paths, tail churn scrubbed),
//! while distinct failures stay distinct.

use crate::qa::fingerprint::{failure_fingerprint, fingerprint_tag, normalized_output_head};

const WS: &str = "polaris";
const CHECK: &str = "lint";
const ROOT: &str = "/home/user/workspace/polaris";

#[test]
fn identical_failures_share_a_fingerprint() {
    let output = "error: broken frontmatter in docs/a.md\nexpected key 'title'";
    let first = failure_fingerprint(WS, CHECK, ROOT, output, "exit 1");
    let second = failure_fingerprint(WS, CHECK, ROOT, output, "exit 1");
    assert_eq!(first, second);
    assert_eq!(first.len(), 12);
    assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
}

#[test]
fn workspace_check_and_output_all_discriminate() {
    let output = "error: broken";
    let base = failure_fingerprint(WS, CHECK, ROOT, output, "exit 1");
    assert_ne!(
        base,
        failure_fingerprint("bridge", CHECK, ROOT, output, "exit 1")
    );
    assert_ne!(
        base,
        failure_fingerprint(WS, "tests", ROOT, output, "exit 1")
    );
    assert_ne!(
        base,
        failure_fingerprint(WS, CHECK, ROOT, "error: different", "exit 1")
    );
}

#[test]
fn ansi_whitespace_and_repo_root_are_normalized_away() {
    let plain = format!("error in {ROOT}/docs/a.md\nline two");
    let noisy = format!("\u{1b}[31merror   in\t{ROOT}/docs/a.md\u{1b}[0m\r\n\n   line   two   \n");
    assert_eq!(
        failure_fingerprint(WS, CHECK, ROOT, &plain, "exit 1"),
        failure_fingerprint(WS, CHECK, ROOT, &noisy, "exit 1"),
    );
    // The other direction: a different repo root must not change the hash.
    assert_eq!(
        failure_fingerprint(WS, CHECK, ROOT, &plain, "exit 1"),
        failure_fingerprint(
            WS,
            CHECK,
            "/srv/polaris",
            &plain.replace(ROOT, "/srv/polaris"),
            "exit 1"
        ),
    );
}

#[test]
fn tail_churn_beyond_the_head_window_is_ignored() {
    let head: String = (0..25).map(|i| format!("error line {i}\n")).collect();
    let with_timing = format!("{head}finished in 12.3s\n");
    let with_other_timing = format!("{head}finished in 45.6s\n");
    assert_eq!(
        failure_fingerprint(WS, CHECK, ROOT, &with_timing, "exit 1"),
        failure_fingerprint(WS, CHECK, ROOT, &with_other_timing, "exit 1"),
    );
}

#[test]
fn silent_failures_fall_back_to_the_exit_summary() {
    let exit_1 = failure_fingerprint(WS, CHECK, ROOT, "", "exit 1");
    let exit_2 = failure_fingerprint(WS, CHECK, ROOT, "", "exit 2");
    let timeout = failure_fingerprint(WS, CHECK, ROOT, "", "timeout after 1800s");
    assert_ne!(exit_1, exit_2);
    assert_ne!(exit_1, timeout);
    // Deterministic: same silent failure, same hash.
    assert_eq!(
        exit_1,
        failure_fingerprint(WS, CHECK, ROOT, "\n  \n", "exit 1")
    );
}

#[test]
fn normalized_head_drops_blank_lines_and_caps_at_twenty() {
    let output = "  a  \n\n\nb\tc\n".to_string() + &"x\n".repeat(40);
    let head = normalized_output_head(&output, ROOT);
    let lines: Vec<&str> = head.lines().collect();
    assert_eq!(lines[0], "a");
    assert_eq!(lines[1], "b c");
    assert_eq!(lines.len(), 20);
}

#[test]
fn tag_shape_is_stable() {
    assert_eq!(fingerprint_tag("abc123def456"), "fp-abc123def456");
}

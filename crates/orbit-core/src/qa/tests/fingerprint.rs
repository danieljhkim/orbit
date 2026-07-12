//! Finding-fingerprint tests [ORB-10039, reworked ORB-10146]: the fingerprint
//! must be stable across reruns of the same finding (so one open task per
//! issue) while staying distinct across workspaces and finding names.

use crate::qa::fingerprint::{finding_fingerprint, fingerprint_tag, normalize_signature};

const WS: &str = "polaris";
const NAME: &str = "login redirect loops on expired session";

#[test]
fn same_finding_hashes_stably() {
    let first = finding_fingerprint(WS, NAME);
    let second = finding_fingerprint(WS, NAME);
    assert_eq!(first, second);
    assert_eq!(first.len(), 12, "12 hex chars");
}

#[test]
fn workspace_and_name_change_the_fingerprint() {
    let base = finding_fingerprint(WS, NAME);
    assert_ne!(base, finding_fingerprint("bridge", NAME));
    assert_ne!(
        base,
        finding_fingerprint(WS, "a completely different issue")
    );
}

#[test]
fn formatting_and_case_differences_collapse() {
    let plain = finding_fingerprint(WS, NAME);
    let noisy = finding_fingerprint(WS, "  Login   Redirect Loops On Expired Session  ");
    assert_eq!(plain, noisy, "whitespace + case normalized");

    let with_ansi = finding_fingerprint(
        WS,
        "\u{1b}[31mlogin redirect loops on expired session\u{1b}[0m",
    );
    assert_eq!(plain, with_ansi, "ANSI escapes stripped");
}

#[test]
fn blank_name_falls_back_to_a_stable_signature() {
    let empty = finding_fingerprint(WS, "");
    let whitespace = finding_fingerprint(WS, "   \n  ");
    assert_eq!(empty, whitespace);
    // Still distinct per workspace.
    assert_ne!(empty, finding_fingerprint("bridge", ""));
}

#[test]
fn fingerprint_tag_prefixes_fp() {
    assert_eq!(fingerprint_tag("abc123"), "fp-abc123");
}

#[test]
fn normalize_signature_lowercases_and_collapses() {
    assert_eq!(normalize_signature("  Foo   BAR  "), "foo bar");
}

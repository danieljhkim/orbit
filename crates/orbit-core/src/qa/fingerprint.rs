//! Failure fingerprints for qa-sweep task dedupe [ORB-10039, reworked
//! ORB-10146].
//!
//! A fingerprint identifies *one distinct finding* so the sweep files one task
//! per open issue, not one per pass (design D4). It is
//! `sha256(workspace \n normalized-finding-name)` truncated to 12 hex chars and
//! carried on the filed task as a `fp-<hash>` tag; the next sweep searches open
//! tasks for that tag before filing again.
//!
//! qa-sweep v2 fingerprints over the *finding name* the QA agent reports (a
//! stable signature for the issue) rather than a shell check's output head, so
//! the same feature regression surfaced across reruns dedupes to one task even
//! as the agent's prose evidence varies.

use sha2::{Digest, Sha256};

/// Truncated hex length of the fingerprint hash.
const FINGERPRINT_HEX_LEN: usize = 12;

/// Tag carried by every qa-sweep-filed task.
pub const QA_SWEEP_TAG: &str = "qa-sweep";

/// The `fp-<hash>` dedupe tag for a fingerprint.
pub fn fingerprint_tag(fingerprint: &str) -> String {
    format!("fp-{fingerprint}")
}

/// Compute the dedupe fingerprint for one reported finding.
///
/// The signature is the workspace plus the finding's normalized name, so the
/// same issue reported on later sweeps (possibly with different evidence prose)
/// dedupes to the same open task. A blank name falls back to a stable literal
/// so nameless findings still fingerprint deterministically per workspace.
pub fn finding_fingerprint(workspace: &str, finding_name: &str) -> String {
    let signature = normalize_signature(finding_name);
    let signature = if signature.is_empty() {
        "<unnamed-finding>"
    } else {
        signature.as_str()
    };
    let digest = Sha256::digest(format!("{workspace}\n{signature}").as_bytes());
    let mut hex = format!("{digest:x}");
    hex.truncate(FINGERPRINT_HEX_LEN);
    hex
}

/// Normalize a finding name to a stable signature: ANSI escapes stripped,
/// lowercased, and internal whitespace collapsed so trivial formatting
/// differences do not split one finding into two fingerprints.
pub(crate) fn normalize_signature(name: &str) -> String {
    collapse_whitespace(&strip_ansi(name)).to_ascii_lowercase()
}

/// Drop ANSI CSI/OSC escape sequences (colors, cursor moves, titles).
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ ... final byte in 0x40..=0x7e
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... terminated by BEL or ESC \
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{07}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Bare two-char escape (ESC c etc.): drop the follower too.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Trim and collapse internal whitespace runs to single spaces.
fn collapse_whitespace(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

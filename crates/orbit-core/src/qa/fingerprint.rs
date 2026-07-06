//! Failure fingerprints for qa-sweep task dedupe [ORB-10039].
//!
//! A fingerprint identifies *one distinct way a check fails* so the sweep
//! files one task per broken check, not one per pass (design D4). It is
//! `sha256(workspace \n check \n normalized-output-head)` truncated to 12 hex
//! chars and carried on the filed task as a `fp-<hash>` tag; the next sweep
//! searches open tasks for that tag before filing again.
//!
//! Normalization aims for *stability across reruns of the same failure*
//! without collapsing genuinely different failures: ANSI escapes and the
//! absolute repo root are scrubbed (they vary per host/terminal), whitespace
//! runs are collapsed, and only the head of the output is hashed (tails often
//! carry timing summaries and counters that churn per run).

use sha2::{Digest, Sha256};

/// Lines of normalized output that participate in the fingerprint.
const FINGERPRINT_HEAD_LINES: usize = 20;
/// Truncated hex length of the fingerprint hash.
const FINGERPRINT_HEX_LEN: usize = 12;

/// Tag carried by every qa-sweep-filed task.
pub const QA_SWEEP_TAG: &str = "qa-sweep";

/// The `fp-<hash>` dedupe tag for a fingerprint.
pub fn fingerprint_tag(fingerprint: &str) -> String {
    format!("fp-{fingerprint}")
}

/// Compute the dedupe fingerprint for one failing check.
///
/// `output` is the check's combined stdout+stderr; `exit_summary` is a stable
/// textual fallback (e.g. `"exit 2"` or `"timeout"`) that keeps silent
/// failures distinguishable by exit mode.
pub fn failure_fingerprint(
    workspace: &str,
    check: &str,
    repo_root: &str,
    output: &str,
    exit_summary: &str,
) -> String {
    let head = normalized_output_head(output, repo_root);
    let signature = if head.is_empty() {
        exit_summary.to_string()
    } else {
        head
    };
    let digest = Sha256::digest(format!("{workspace}\n{check}\n{signature}").as_bytes());
    let mut hex = format!("{digest:x}");
    hex.truncate(FINGERPRINT_HEX_LEN);
    hex
}

/// Normalize check output down to the stable head used as failure signature.
pub(crate) fn normalized_output_head(output: &str, repo_root: &str) -> String {
    strip_ansi(output)
        .replace(repo_root, "<repo>")
        .lines()
        .map(collapse_whitespace)
        .filter(|line| !line.is_empty())
        .take(FINGERPRINT_HEAD_LINES)
        .collect::<Vec<_>>()
        .join("\n")
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

/// Trim the line and collapse internal whitespace runs to single spaces.
fn collapse_whitespace(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

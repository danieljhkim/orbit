//! The untrusted half of audit attribution: what a caller says it is.
//!
//! Orbit's trusted identity (`audit_events.role`, and the canonical actor
//! projection derived from it) is only populated when the caller arrived
//! through a path Orbit can authenticate — a managed run envelope or the local
//! CLI. An MCP server started from a client's own config has no such envelope,
//! so its `role` is `unverified` and its canonical actor is `unattributed`.
//!
//! That is the correct label, and this module does not widen it. It adds a
//! *second, separate* field for the identity the caller claimed about itself,
//! so unauthenticated traffic is attributable-but-labelled instead of
//! anonymous [ORB-10890].
//!
//! ## The one rule
//!
//! A self-reported value is evidence, never a credential. It is stored in its
//! own column, never merged into `role` or the `actor_*` projection, never
//! consulted by an authentication or authorization decision, and never
//! rendered without being marked unverified. Anyone who can reach the MCP
//! server can supply any string here; a surface that forgets the marking turns
//! a published per-agent number into a forgeable one.
//!
//! ## Absent is not a default
//!
//! [`normalize_self_reported_actor`] returns `None` for a missing, blank, or
//! malformed claim. `None` means anonymous. It must never be filled in from a
//! neighbouring session, a previous call, or the trusted label — inheriting
//! another actor's identity is exactly the failure this field exists to make
//! visible.

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Longest self-reported actor Orbit will record.
///
/// Generous for a client name or model string (`claude-code`, `claude-opus-5`)
/// and far short of anything that could be used to smuggle a payload through
/// the audit log.
pub const SELF_REPORTED_ACTOR_MAX_LEN: usize = 128;

/// The label an aggregate reports for traffic that claimed no identity at all.
pub const ANONYMOUS_ACTOR_LABEL: &str = "anonymous";

/// How an audit row's actor label was established.
///
/// This is the trust dimension, kept separate from *who* the actor is so a
/// caller can ask for authenticated-only, self-reported-only, or combined
/// counts without string-matching a label.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum AuditAttribution {
    /// Orbit authenticated the caller: a managed run envelope, or a local CLI
    /// invocation whose identity Orbit itself supplied.
    Authenticated,
    /// The caller named itself and Orbit could not verify the claim. Counts
    /// belong in their own denominator, never folded into `Authenticated`.
    SelfReported,
    /// Neither a trusted identity nor a usable claim.
    Anonymous,
}

impl AuditAttribution {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditAttribution::Authenticated => "authenticated",
            AuditAttribution::SelfReported => "self_reported",
            AuditAttribution::Anonymous => "anonymous",
        }
    }

    /// True when this row's actor may be used as an authenticated principal.
    /// Only [`AuditAttribution::Authenticated`] qualifies.
    pub fn is_authenticated(self) -> bool {
        matches!(self, AuditAttribution::Authenticated)
    }
}

impl Display for AuditAttribution {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuditAttribution {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "authenticated" => Ok(AuditAttribution::Authenticated),
            "self_reported" => Ok(AuditAttribution::SelfReported),
            "anonymous" => Ok(AuditAttribution::Anonymous),
            other => Err(format!("unknown audit attribution: {other}")),
        }
    }
}

/// Reduce a caller-supplied identity claim to the bounded form Orbit records,
/// or `None` when there is nothing usable to record.
///
/// Rejects (as anonymous, never as an error the caller can act on):
/// - a claim that is empty or only whitespace,
/// - a claim containing a control character — newlines and tabs would let a
///   claim forge extra fields in any line-oriented rendering of the audit log,
/// - a claim longer than [`SELF_REPORTED_ACTOR_MAX_LEN`] after trimming.
///
/// Accepted claims are lowercased and have internal whitespace runs collapsed
/// to a single space, so `Claude  Code` and `claude code` aggregate as one
/// group. Case folding is safe here precisely because the value is never
/// compared against a trusted label: two claims colliding after normalization
/// are both still marked self-reported.
pub fn normalize_self_reported_actor(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > SELF_REPORTED_ACTOR_MAX_LEN {
        return None;
    }
    if trimmed.chars().any(char::is_control) {
        return None;
    }

    let normalized = trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // `split_whitespace` on a non-empty, non-control string always yields at
    // least one token, but re-checking keeps the postcondition local: this
    // function never returns `Some("")`.
    (!normalized.is_empty()).then_some(normalized)
}

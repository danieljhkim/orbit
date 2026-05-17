//! Unified secret redaction.
//!
//! Consolidates the three surfaces scattered across the workspace today:
//! - `orbit_common::types` imports that previously relied on the legacy leaf
//!   crate's root re-exports for env-value scrubbing
//! - `orbit_agent::loop_engine::audit::redaction::RedactionMiddleware` —
//!   regex-based patterns for `Authorization` / `x-api-key` / `Bearer` in
//!   HTTP-shaped payloads (headers, JSON)
//! - `orbit_engine::activity_job::cli_runner::ArgvRedactor` — the above plus a raw
//!   `sk-…` pattern for argv that leaks provider keys
//!
//! This module is the single source of truth for generic, domain-free
//! redaction, including the `OrbitError`-aware helper now that both the
//! utilities and domain types live in the same crate.
//!
//! Callers pick the layer they need:
//! - [`redact_sensitive_env_text`] — scrub live env-var values from a string
//! - [`PatternRedactor`] — regex pattern scrubbing (HTTP / argv / JSON)
//! - [`redact_all`] — env + default patterns in one pass (use when you don't
//!   know what shape the input has and want maximum coverage)
//! - [`is_high_confidence_credential_token`] — exact-token refusal heuristic
//!   for write boundaries that should reject clear credential values instead
//!   of masking them silently

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::{borrow::Cow, sync::OnceLock};

use regex::Regex;
use serde_json::Value;

use crate::types::OrbitError;

const REDACTED_ENV_VALUE: &str = "[REDACTED_ENV]";
static DEFAULT_PATTERN_REDACTOR: OnceLock<PatternRedactor> = OnceLock::new();
static OPENAI_KEY_PATTERN: OnceLock<Regex> = OnceLock::new();
static GITHUB_TOKEN_PATTERN: OnceLock<Regex> = OnceLock::new();
static SLACK_TOKEN_PATTERN: OnceLock<Regex> = OnceLock::new();

// ---------------------------------------------------------------------------
// Env-var value scrubbing
// ---------------------------------------------------------------------------

/// Replace occurrences of any sensitive env-var value (as seen in the live
/// process environment) with `[REDACTED_ENV]`.
///
/// "Sensitive" is matched against the var *name* — anything containing
/// SECRET / TOKEN / PASSWORD / API_KEY / etc. See [`is_sensitive_env_name`].
pub fn redact_sensitive_env_text(raw: &str) -> String {
    let mut redacted = raw.to_string();
    for secret in sensitive_env_values() {
        redacted = redacted.replace(&secret, REDACTED_ENV_VALUE);
    }
    redacted
}

pub fn redact_sensitive_env_option(raw: Option<String>) -> Option<String> {
    raw.map(|value| redact_sensitive_env_text(&value))
}

pub fn redact_sensitive_env_json(value: Value) -> Value {
    match value {
        Value::String(raw) => Value::String(redact_sensitive_env_text(&raw)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_sensitive_env_json).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_sensitive_env_json(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Replace `$HOME` / `$USERPROFILE` with `~` in the given string. Prevents
/// user-identifiable paths from leaking into logs. Addresses CodeQL
/// `rust/cleartext-logging`.
pub fn redact_home_dir(text: &str) -> String {
    if let Some(home) = home_dir_string() {
        text.replace(&home, "~")
    } else {
        text.to_string()
    }
}

/// Apply env-value redaction to the message carried by any `OrbitError` variant.
pub fn redact_sensitive_env_error(error: OrbitError) -> OrbitError {
    match error {
        OrbitError::PolicyDenied(m) => OrbitError::PolicyDenied(redact_sensitive_env_text(&m)),
        OrbitError::NotFound { kind, id } => OrbitError::NotFound {
            kind,
            id: redact_sensitive_env_text(&id),
        },
        OrbitError::TaskApprovalRequired(m) => {
            OrbitError::TaskApprovalRequired(redact_sensitive_env_text(&m))
        }
        OrbitError::AdrInvalidTransition(m) => {
            OrbitError::AdrInvalidTransition(redact_sensitive_env_text(&m))
        }
        OrbitError::CompanionNotInstalled(m) => {
            OrbitError::CompanionNotInstalled(redact_sensitive_env_text(&m))
        }
        OrbitError::InvalidInput(m) => OrbitError::InvalidInput(redact_sensitive_env_text(&m)),
        OrbitError::InvalidInputDiagnostic {
            message,
            did_you_mean,
        } => OrbitError::InvalidInputDiagnostic {
            message: redact_sensitive_env_text(&message),
            did_you_mean: did_you_mean
                .into_iter()
                .map(|suggestion| redact_sensitive_env_text(&suggestion))
                .collect(),
        },
        OrbitError::SensitiveInput(m) => OrbitError::SensitiveInput(redact_sensitive_env_text(&m)),
        OrbitError::SkillValidation(m) => {
            OrbitError::SkillValidation(redact_sensitive_env_text(&m))
        }
        OrbitError::JobValidation(m) => OrbitError::JobValidation(redact_sensitive_env_text(&m)),
        OrbitError::AgentProtocolViolation(m) => {
            OrbitError::AgentProtocolViolation(redact_sensitive_env_text(&m))
        }
        OrbitError::UnsupportedAgentProvider(m) => {
            OrbitError::UnsupportedAgentProvider(redact_sensitive_env_text(&m))
        }
        OrbitError::Execution(m) => OrbitError::Execution(redact_sensitive_env_text(&m)),
        OrbitError::Store(m) => OrbitError::Store(redact_sensitive_env_text(&m)),
        OrbitError::TaskStatusTransition(m) => {
            OrbitError::TaskStatusTransition(redact_sensitive_env_text(&m))
        }
        OrbitError::JobRunStateTransition(m) => {
            OrbitError::JobRunStateTransition(redact_sensitive_env_text(&m))
        }
        OrbitError::Io(m) => OrbitError::Io(redact_sensitive_env_text(&m)),
        OrbitError::WorkspaceError(m) => OrbitError::WorkspaceError(redact_sensitive_env_text(&m)),
        OrbitError::Migration(m) => OrbitError::Migration(redact_sensitive_env_text(&m)),
    }
}

fn home_dir_string() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .filter(|h| !h.is_empty())
}

fn sensitive_env_values() -> Vec<String> {
    let mut values = std::env::vars()
        .filter(|(name, value)| is_sensitive_env_name(name) && is_redactable_value(value))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

fn is_redactable_value(value: &str) -> bool {
    value.trim().len() >= 4
}

pub fn is_sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("PASSCODE")
        || upper.contains("API_KEY")
        || upper.ends_with("_KEY")
        || upper.contains("PRIVATE")
        || upper.contains("CREDENTIAL")
        || upper.contains("COOKIE")
        || upper.contains("SESSION")
        || upper.contains("BEARER")
        || upper.contains("AUTH")
}

// ---------------------------------------------------------------------------
// Pattern-based redaction (HTTP + argv)
// ---------------------------------------------------------------------------

/// Regex-driven scrubber for HTTP-shape and credential-shaped secrets.
///
/// Builds to `default()` give you the same coverage as the former
/// `RedactionMiddleware` (Authorization / x-api-key / Bearer / raw header
/// lines) plus high-confidence provider tokens that may appear inside prose.
/// Use [`PatternRedactor::with_argv_secrets`] when scrubbing subprocess argv;
/// it retains compatibility with the old argv-specific constructor.
pub struct PatternRedactor {
    patterns: Vec<(Regex, &'static str)>,
}

impl PatternRedactor {
    /// Authorization / x-api-key / api_key / Bearer in both JSON and
    /// raw-header form, plus common provider token shapes that may appear
    /// inside free text.
    pub fn http_default() -> Self {
        let patterns = vec![
            (
                Regex::new(r#"(?i)"authorization"\s*:\s*"[^"]*""#).expect("valid regex"),
                r#""authorization":"[REDACTED_AUTH]""#,
            ),
            (
                Regex::new(r#"(?i)"x-api-key"\s*:\s*"[^"]*""#).expect("valid regex"),
                r#""x-api-key":"[REDACTED_AUTH]""#,
            ),
            (
                Regex::new(r#"(?i)"api[_-]?key"\s*:\s*"[^"]*""#).expect("valid regex"),
                r#""api_key":"[REDACTED_AUTH]""#,
            ),
            (
                Regex::new(r#"(?i)bearer\s+[A-Za-z0-9._\-+/=]+"#).expect("valid regex"),
                "Bearer [REDACTED_AUTH]",
            ),
            (
                Regex::new(r"(?im)^(\s*authorization\s*:\s*).+$").expect("valid regex"),
                "${1}[REDACTED_AUTH]",
            ),
            (
                Regex::new(r"(?im)^(\s*x-api-key\s*:\s*).+$").expect("valid regex"),
                "${1}[REDACTED_AUTH]",
            ),
            (
                Regex::new(r"(?im)^(\s*api[_-]?key\s*:\s*).+$").expect("valid regex"),
                "${1}[REDACTED_AUTH]",
            ),
            (
                Regex::new(r"sk-[A-Za-z0-9_\-]{20,}").expect("valid regex"),
                "[REDACTED_API_KEY]",
            ),
            (
                Regex::new(r"ghp_[A-Za-z0-9]{36}").expect("valid regex"),
                "[REDACTED_API_KEY]",
            ),
            (
                Regex::new(r"xox[baprs]-[A-Za-z0-9\-]{10,}").expect("valid regex"),
                "[REDACTED_API_KEY]",
            ),
        ];
        Self { patterns }
    }

    /// HTTP defaults plus a bare `sk-…` token pattern suitable for scrubbing
    /// CLI argv where a provider key occasionally ends up as a flag value.
    pub fn with_argv_secrets() -> Self {
        let mut me = Self::http_default();
        me.patterns.push((
            Regex::new(r"sk-[A-Za-z0-9_\-]+").expect("valid regex"),
            "[REDACTED_API_KEY]",
        ));
        me
    }

    pub fn empty() -> Self {
        Self { patterns: vec![] }
    }

    /// Apply all patterns in order to `input`.
    pub fn apply_str(&self, input: &str) -> String {
        let mut out: Cow<'_, str> = Cow::Borrowed(input);
        for (pattern, replacement) in &self.patterns {
            match pattern.replace_all(&out, *replacement) {
                Cow::Borrowed(_) => {}
                Cow::Owned(new) => out = Cow::Owned(new),
            }
        }
        out.into_owned()
    }

    /// Byte-level convenience for callers holding raw HTTP bodies. Non-UTF-8
    /// input is returned unchanged.
    pub fn apply_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        match std::str::from_utf8(bytes) {
            Ok(text) => self.apply_str(text).into_bytes(),
            Err(_) => bytes.to_vec(),
        }
    }
}

impl Default for PatternRedactor {
    fn default() -> Self {
        Self::http_default()
    }
}

// ---------------------------------------------------------------------------
// Combined
// ---------------------------------------------------------------------------

/// Apply env-value and default HTTP pattern redaction in one pass. Use when
/// the input shape is unknown (log lines, aggregated error messages).
pub fn redact_all(input: &str) -> String {
    let env_scrubbed = redact_sensitive_env_text(input);
    default_pattern_redactor().apply_str(&env_scrubbed)
}

pub(crate) fn default_pattern_redactor() -> &'static PatternRedactor {
    DEFAULT_PATTERN_REDACTOR.get_or_init(PatternRedactor::http_default)
}

/// Returns `true` when the whole trimmed value is a known credential token.
///
/// Callers use this to refuse writes where the entire field is plainly a
/// credential. The same token embedded in larger prose is left to
/// [`redact_all`] so the surrounding context can be preserved.
pub fn is_high_confidence_credential_token(input: &str) -> bool {
    let trimmed = input.trim();
    openai_key_pattern().is_match(trimmed)
        || github_token_pattern().is_match(trimmed)
        || slack_token_pattern().is_match(trimmed)
}

fn openai_key_pattern() -> &'static Regex {
    OPENAI_KEY_PATTERN.get_or_init(|| Regex::new(r"^sk-[A-Za-z0-9_\-]{20,}$").expect("valid regex"))
}

fn github_token_pattern() -> &'static Regex {
    GITHUB_TOKEN_PATTERN.get_or_init(|| Regex::new(r"^ghp_[A-Za-z0-9]{36}$").expect("valid regex"))
}

fn slack_token_pattern() -> &'static Regex {
    SLACK_TOKEN_PATTERN
        .get_or_init(|| Regex::new(r"^xox[baprs]-[A-Za-z0-9\-]{10,}$").expect("valid regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_all_masks_embedded_provider_tokens() {
        let input = concat!(
            "OpenAI sk-0123456789abcdefghijklmn ",
            "GitHub ghp_012345678901234567890123456789012345 ",
            "Slack xoxb-0123456789-abcd"
        );

        let redacted = redact_all(input);

        assert!(!redacted.contains("sk-0123456789abcdefghijklmn"));
        assert!(!redacted.contains("ghp_012345678901234567890123456789012345"));
        assert!(!redacted.contains("xoxb-0123456789-abcd"));
        assert!(redacted.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn high_confidence_credential_detection_is_whole_token_only() {
        assert!(is_high_confidence_credential_token(
            "sk-0123456789abcdefghijklmn"
        ));
        assert!(is_high_confidence_credential_token(
            "ghp_012345678901234567890123456789012345"
        ));
        assert!(is_high_confidence_credential_token("xoxb-0123456789-abcd"));
        assert!(!is_high_confidence_credential_token(
            "token sk-0123456789abcdefghijklmn in prose"
        ));
    }

    #[test]
    fn redact_all_is_idempotent_for_fixture_inputs() {
        let fixtures = [
            "Authorization: Bearer abcdef012345",
            r#"{"api_key":"abcdef012345"}"#,
            "sk-0123456789abcdefghijklmn",
            "already [REDACTED_ENV] and [REDACTED_API_KEY]",
        ];

        for fixture in fixtures {
            assert_eq!(redact_all(&redact_all(fixture)), redact_all(fixture));
        }
    }
}

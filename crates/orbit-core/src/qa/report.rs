//! The structured findings report a QA agent emits as its final output
//! [ORB-10146].
//!
//! The QA prompt (see [`super::prompt`]) instructs the agent to end with a JSON
//! document `{"findings": [{name, severity, summary, evidence, commits}]}` — an
//! empty array for a clean pass. Agents wrap that JSON in prose or a ```json
//! fence more often than not, so parsing is deliberately lenient about the
//! surrounding text but strict about the contract: a terminal run whose output
//! carries no parseable `findings` object is a *bad report*, and the sweep
//! holds the watermark on it rather than recording a silent green.

use orbit_common::types::TaskPriority;
use serde::Deserialize;
use serde_json::Value;

/// A parsed QA agent report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaReport {
    /// Findings the agent reported; empty for a clean pass.
    pub findings: Vec<Finding>,
}

/// One issue the QA agent surfaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable signature for the issue; the dedupe fingerprint hashes it.
    pub name: String,
    /// Reported severity, mapped to a task priority (clamped by policy later).
    pub severity: Severity,
    /// One-line description of the issue.
    pub summary: String,
    /// Evidence the agent gathered (repro steps, output, reasoning).
    pub evidence: String,
    /// Commits the agent attributes the issue to.
    pub commits: Vec<String>,
}

/// A finding's reported severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    /// Missing or unrecognized severity; the sweep falls back to the configured
    /// default priority.
    Unknown,
}

impl Severity {
    /// Lenient parse: case-insensitive, trims, maps a few common synonyms.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "critical" | "crit" | "blocker" => Self::Critical,
            "high" | "major" => Self::High,
            "medium" | "moderate" | "normal" => Self::Medium,
            "low" | "minor" | "trivial" => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// The task priority this severity maps to, before policy clamping.
    /// `Unknown` yields `None` so the caller substitutes the default priority.
    pub fn mapped_priority(self) -> Option<TaskPriority> {
        match self {
            Self::Critical => Some(TaskPriority::Critical),
            Self::High => Some(TaskPriority::High),
            Self::Medium => Some(TaskPriority::Medium),
            Self::Low => Some(TaskPriority::Low),
            Self::Unknown => None,
        }
    }

    /// Lowercase label for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }
}

/// Rank a priority for clamping (higher = more severe).
fn priority_rank(priority: TaskPriority) -> u8 {
    match priority {
        TaskPriority::Low => 0,
        TaskPriority::Medium => 1,
        TaskPriority::High => 2,
        TaskPriority::Critical => 3,
    }
}

/// Resolve a finding's task priority: map its severity (default priority when
/// unknown), then clamp so it never exceeds the configured ceiling.
pub(crate) fn resolve_priority(severity: Severity, ceiling: TaskPriority) -> TaskPriority {
    let mapped = severity.mapped_priority().unwrap_or(ceiling);
    if priority_rank(mapped) > priority_rank(ceiling) {
        ceiling
    } else {
        mapped
    }
}

/// Why a report failed to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportParseError {
    pub reason: String,
}

impl std::fmt::Display for ReportParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason)
    }
}

#[derive(Debug, Deserialize)]
struct RawReport {
    findings: Option<Vec<RawFinding>>,
}

#[derive(Debug, Deserialize)]
struct RawFinding {
    name: Option<String>,
    severity: Option<String>,
    summary: Option<String>,
    evidence: Option<String>,
    commits: Option<Vec<Value>>,
}

/// Parse a QA agent's final output into a [`QaReport`].
///
/// Accepts the JSON document bare, inside a ```json / ``` fence, or embedded in
/// surrounding prose (the first balanced `{...}` object that deserializes and
/// carries a `findings` array wins). Returns an error when no such object is
/// present — the contract requires an explicit `findings` array even for a
/// clean pass.
pub fn parse_report(raw: &str) -> Result<QaReport, ReportParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ReportParseError {
            reason: "empty agent output; no findings report".to_string(),
        });
    }

    for candidate in json_candidates(trimmed) {
        if let Ok(report) = serde_json::from_str::<RawReport>(&candidate)
            && let Some(raw_findings) = report.findings
        {
            return Ok(QaReport {
                findings: raw_findings.into_iter().map(Finding::from_raw).collect(),
            });
        }
    }

    Err(ReportParseError {
        reason: "no parseable {\"findings\": [...]} object in agent output".to_string(),
    })
}

impl Finding {
    fn from_raw(raw: RawFinding) -> Self {
        Self {
            name: raw.name.unwrap_or_default().trim().to_string(),
            severity: raw
                .severity
                .as_deref()
                .map(Severity::parse)
                .unwrap_or(Severity::Unknown),
            summary: raw.summary.unwrap_or_default().trim().to_string(),
            evidence: raw.evidence.unwrap_or_default().trim().to_string(),
            commits: raw
                .commits
                .unwrap_or_default()
                .into_iter()
                .map(|value| match value {
                    Value::String(string) => string,
                    other => other.to_string(),
                })
                .filter(|commit| !commit.trim().is_empty())
                .collect(),
        }
    }
}

/// Ordered JSON candidates to try: fenced blocks first, then the whole string,
/// then the outermost balanced object embedded in prose.
fn json_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    candidates.extend(fenced_blocks(text));
    candidates.push(text.to_string());
    if let Some(object) = outermost_object(text) {
        candidates.push(object);
    }
    candidates
}

/// Extract the contents of ``` fenced code blocks (```json or bare ```).
fn fenced_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after_open = &rest[open + 3..];
        // Skip an optional language tag up to the newline.
        let body_start = match after_open.find('\n') {
            Some(newline) => newline + 1,
            None => break,
        };
        let body = &after_open[body_start..];
        let Some(close) = body.find("```") else {
            break;
        };
        blocks.push(body[..close].trim().to_string());
        rest = &body[close + 3..];
    }
    blocks
}

/// The substring from the first `{` to the last `}` inclusive, if both exist.
fn outermost_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        Some(text[start..=end].to_string())
    } else {
        None
    }
}

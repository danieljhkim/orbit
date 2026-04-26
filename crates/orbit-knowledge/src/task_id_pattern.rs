//! Configurable task-ID pattern (T20260426-0507).
//!
//! Replaces the formerly-hardcoded Orbit regex `\[T\d{8}-\d{4}(?:-\d+)?\]` with
//! a validated wrapper that other codebases can swap for their own conventions
//! (Jira `PROJ-123`, Linear `ENG-123`, GitHub `#123`, …).
//!
//! Capture-group convention: when the pattern has at least one capture group,
//! group 1 is the task ID. Otherwise the whole match is the ID. This lets the
//! Orbit default strip the surrounding brackets via `\[(T...)\]` instead of a
//! bespoke post-match string slice.

use regex::Regex;

use crate::error::KnowledgeError;

/// Default Orbit task-ID pattern. Capture group 1 is the bare ID
/// (`T20260421-0528` rather than `[T20260421-0528]`).
pub const DEFAULT_TASK_ID_PATTERN: &str = r"\[(T\d{8}-\d{4}(?:-\d+)?)\]";

/// Validated task-ID extraction pattern.
///
/// Construct via [`TaskIdPattern::default`] for the Orbit format or
/// [`TaskIdPattern::new`] with a user-supplied regex string. `new` compiles
/// eagerly so invalid regexes surface at parse/load time, not on first match.
#[derive(Debug, Clone)]
pub struct TaskIdPattern {
    regex: Regex,
    source: String,
}

impl TaskIdPattern {
    /// Build a pattern from a regex string. Errors when the regex fails to
    /// compile.
    pub fn new(source: impl Into<String>) -> Result<Self, KnowledgeError> {
        let source = source.into();
        let regex = Regex::new(&source).map_err(|error| {
            KnowledgeError::invalid_data(format!("invalid task-ID regex `{source}`: {error}"))
        })?;
        Ok(Self { regex, source })
    }

    /// Original regex string, suitable for serialization (graph manifest,
    /// debug output).
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Extract task IDs from a commit message. Sorted, deduplicated.
    ///
    /// Capture-group convention:
    /// - With ≥1 capture group: group 1 is the ID.
    /// - With no capture groups: the whole match is the ID.
    pub fn extract_ids(&self, message: &str) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        let has_capture_groups = self.regex.captures_len() > 1;
        if has_capture_groups {
            for caps in self.regex.captures_iter(message) {
                if let Some(m) = caps.get(1) {
                    ids.push(m.as_str().to_string());
                }
            }
        } else {
            for m in self.regex.find_iter(message) {
                ids.push(m.as_str().to_string());
            }
        }
        ids.sort();
        ids.dedup();
        ids
    }
}

impl Default for TaskIdPattern {
    fn default() -> Self {
        Self::new(DEFAULT_TASK_ID_PATTERN).expect("default Orbit task-ID pattern must compile")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pattern_extracts_orbit_ids_with_brackets_stripped() {
        let pattern = TaskIdPattern::default();
        assert_eq!(
            pattern.extract_ids("[T20260421-0528] add task_ids"),
            vec!["T20260421-0528"]
        );
    }

    #[test]
    fn default_pattern_handles_amended_suffix() {
        let pattern = TaskIdPattern::default();
        assert_eq!(
            pattern.extract_ids("[T20260421-0528-2] incremental fix"),
            vec!["T20260421-0528-2"]
        );
    }

    #[test]
    fn default_pattern_deduplicates_and_sorts() {
        let pattern = TaskIdPattern::default();
        let got = pattern
            .extract_ids("[T20260421-0528] first\n[T20260421-0342] second\n[T20260421-0528] dup");
        assert_eq!(got, vec!["T20260421-0342", "T20260421-0528"]);
    }

    #[test]
    fn default_pattern_ignores_malformed_tags() {
        let pattern = TaskIdPattern::default();
        assert_eq!(
            pattern.extract_ids("[T1234] wrong shape\n[T20260421-0528] right"),
            vec!["T20260421-0528"]
        );
    }

    #[test]
    fn default_pattern_empty_on_no_tags() {
        let pattern = TaskIdPattern::default();
        assert!(pattern.extract_ids("merge pull request #42").is_empty());
    }

    #[test]
    fn jira_pattern_uses_whole_match_when_no_capture_groups() {
        let pattern = TaskIdPattern::new(r"[A-Z]+-\d+").expect("jira regex compiles");
        assert_eq!(pattern.extract_ids("PROJ-123: fix thing"), vec!["PROJ-123"]);
    }

    #[test]
    fn jira_pattern_captures_multiple_ids() {
        let pattern = TaskIdPattern::new(r"[A-Z]+-\d+").expect("jira regex compiles");
        let got = pattern.extract_ids("PROJ-123 closes ENG-7 (refs ENG-7 again)");
        assert_eq!(got, vec!["ENG-7", "PROJ-123"]);
    }

    #[test]
    fn pattern_with_capture_group_uses_group_1() {
        let pattern = TaskIdPattern::new(r"#(\d+)").expect("regex compiles");
        assert_eq!(pattern.extract_ids("Closes #42 and #99"), vec!["42", "99"]);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let err = TaskIdPattern::new("[unclosed").expect_err("invalid regex must error");
        assert!(
            err.reason.contains("invalid task-ID regex"),
            "unexpected reason: {}",
            err.reason
        );
    }

    #[test]
    fn as_str_returns_original_source() {
        let pattern = TaskIdPattern::new(r"[A-Z]+-\d+").expect("regex compiles");
        assert_eq!(pattern.as_str(), r"[A-Z]+-\d+");
    }

    #[test]
    fn default_as_str_matches_constant() {
        assert_eq!(TaskIdPattern::default().as_str(), DEFAULT_TASK_ID_PATTERN);
    }
}

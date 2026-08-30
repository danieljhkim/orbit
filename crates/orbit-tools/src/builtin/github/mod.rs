use orbit_common::OrbitError;
use orbit_common::security::redaction::redact_all;
use orbit_exec::{EnvironmentMode, ExecRequest, StdinMode};
use orbit_types::tool::{ToolParam, ToolSchema};
use serde_json::Value;

use crate::{ToolRegistry, require_str};

pub fn gh_exec_request(
    args: Vec<String>,
    current_dir: Option<String>,
    timeout_ms: u64,
) -> ExecRequest {
    ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir,
        timeout_ms: Some(timeout_ms),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    }
}

pub(super) fn gh_schema(name: &str, description: &str, parameters: Vec<ToolParam>) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        builtin: true,
    }
}

pub(super) fn tool_param(
    name: &str,
    description: &str,
    param_type: &str,
    required: bool,
) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required,
    }
}

macro_rules! gh_tool {
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        request: |$request_ctx:ident, $request_input:ident| $request:block
        response: |$response_ctx:ident, $response_input:ident, $result:ident| $response:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_types::tool::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::OrbitError> {
                let req = {
                    let $request_ctx = ctx;
                    let $request_input = &input;
                    $request
                }?;
                let exec_result = orbit_exec::run_process(&req, &orbit_exec::NoSandbox)?;
                let $response_ctx = ctx;
                let $response_input = &input;
                let $result = &exec_result;
                $response
            }
        }
    };
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        execute: |$execute_ctx:ident, $execute_input:ident| $execute:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_types::tool::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::OrbitError> {
                let $execute_ctx = ctx;
                let $execute_input = input;
                $execute
            }
        }
    };
}

pub(super) use gh_tool;

pub mod auth;
pub mod pr_checkout;
pub mod pr_checks;
pub mod pr_close;
pub mod pr_list;
pub mod repo;
pub mod run_list;
pub mod run_logs;
pub mod run_view;

/// The read-only GitHub discovery surface.
///
/// These five tools are the only sanctioned way for a task body to enumerate
/// CI state: `gh` runs here, as a child of whichever process executes the
/// tool, and its output is redacted and bounded on the way back out. A body
/// that shells out to `gh` itself gets none of that.
///
/// Nothing that mutates GitHub is registered. `pr_checkout`, `pr_checks`,
/// `pr_close`, and `repo` stay unregistered — the PR pipeline drives those
/// operations directly from `orbit-engine`.
pub fn register(registry: &mut ToolRegistry) {
    registry.register(auth::GithubAuthStatusTool);
    registry.register(pr_list::GithubPrListTool);
    registry.register(run_list::GithubRunListTool);
    registry.register(run_logs::GithubRunLogsTool);
    registry.register(run_view::GithubRunViewTool);
}

/// Extract a non-empty `pr` field from the tool input.
/// Accepts a numeric PR number or a GitHub PR URL (extracts the number from the path).
pub(super) fn require_pr(input: &Value) -> Result<String, OrbitError> {
    let pr = require_str(input, "pr")?;
    // Already numeric — use directly.
    if !pr.is_empty() && pr.chars().all(|c| c.is_ascii_digit()) {
        return Ok(pr);
    }
    // Try to extract PR number from a GitHub URL like
    // https://github.com/owner/repo/pull/123
    if pr.contains("github.com/")
        && pr.contains("/pull/")
        && let Some(num) = pr.rsplit('/').next()
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        return Ok(num.to_string());
    }
    Err(OrbitError::InvalidInput(format!(
        "invalid `pr`: \"{pr}\"; must be a numeric PR number or GitHub PR URL"
    )))
}

/// Extract a required numeric GitHub identifier (a workflow-run or job ID).
///
/// Numeric-only is a hardening rule, not a formatting preference: the value is
/// appended to a `gh` argv, and a leading `-` would otherwise be parsed as a
/// flag.
pub(super) fn require_numeric_id(input: &Value, key: &str) -> Result<String, OrbitError> {
    let raw = require_str(input, key)?;
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return Ok(raw);
    }
    Err(OrbitError::InvalidInput(format!(
        "invalid `{key}`: \"{raw}\"; must be a numeric GitHub identifier"
    )))
}

/// Append `--repo <owner/name>` when the caller supplied one.
pub(super) fn push_repo_flag(args: &mut Vec<String>, input: &Value) -> Result<(), OrbitError> {
    push_optional_flag(args, input, "repo", "--repo")
}

/// Append `<flag> <value>` when `key` is present, rejecting a value that would
/// be read as another `gh` flag.
pub(super) fn push_optional_flag(
    args: &mut Vec<String>,
    input: &Value,
    key: &str,
    flag: &str,
) -> Result<(), OrbitError> {
    let Some(value) = input.get(key).and_then(Value::as_str) else {
        return Ok(());
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if value.starts_with('-') {
        return Err(OrbitError::InvalidInput(format!(
            "invalid `{key}`: \"{value}\"; must not start with `-`"
        )));
    }
    args.push(flag.to_string());
    args.push(value.to_string());
    Ok(())
}

/// Read an optional positive bound, clamped into `[1, max]`.
pub(super) fn bounded_limit(
    input: &Value,
    key: &str,
    default: u64,
    max: u64,
) -> Result<u64, OrbitError> {
    let Some(value) = input.get(key) else {
        return Ok(default);
    };
    let raw = match value {
        Value::Number(number) => number.as_u64().ok_or_else(|| {
            OrbitError::InvalidInput(format!("`{key}` must be a positive integer"))
        })?,
        Value::String(text) => text.trim().parse::<u64>().map_err(|error| {
            OrbitError::InvalidInput(format!("`{key}` must be a positive integer: {error}"))
        })?,
        Value::Null => return Ok(default),
        _ => {
            return Err(OrbitError::InvalidInput(format!(
                "`{key}` must be a positive integer"
            )));
        }
    };
    if raw == 0 {
        return Err(OrbitError::InvalidInput(format!(
            "`{key}` must be greater than zero"
        )));
    }
    Ok(raw.min(max))
}

/// Parse a `gh --json` payload, naming the command in the failure.
pub fn parse_gh_json(stdout: &str, label: &str) -> Result<Value, OrbitError> {
    serde_json::from_str(stdout)
        .map_err(|error| OrbitError::Execution(format!("failed to parse {label} output: {error}")))
}

/// One log excerpt, already redacted and bounded.
pub struct BoundedLog {
    pub text: String,
    pub truncated: bool,
    pub total_bytes: usize,
    pub returned_bytes: usize,
}

/// Cap a log excerpt at `max_bytes`, keeping the head and the tail.
///
/// A failed-step log carries its signal at both ends — the head names the
/// command and its arguments, the tail carries the assertion or exit status —
/// so a plain prefix truncation loses the part the reader came for. The gap is
/// marked inline, and `truncated` lets a caller ask for more rather than
/// silently reasoning over a partial log.
pub fn bound_log_text(raw: &str, max_bytes: usize) -> BoundedLog {
    let redacted = redact_all(raw);
    let total_bytes = redacted.len();
    if total_bytes <= max_bytes {
        return BoundedLog {
            returned_bytes: total_bytes,
            text: redacted,
            truncated: false,
            total_bytes,
        };
    }

    let head_len = floor_char_boundary(&redacted, max_bytes / 2);
    let tail_start =
        ceil_char_boundary(&redacted, total_bytes.saturating_sub(max_bytes - head_len));
    let omitted = tail_start - head_len;
    let text = format!(
        "{}\n[... {omitted} bytes omitted; raise the byte budget for more ...]\n{}",
        &redacted[..head_len],
        &redacted[tail_start..]
    );
    BoundedLog {
        returned_bytes: head_len + (total_bytes - tail_start),
        text,
        truncated: true,
        total_bytes,
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Prose a runner emits around a checkout. Matched case-insensitively.
const CHECKOUT_MARKERS: &[&str] = &[
    "head is now at",
    "-> fetch_head",
    "git checkout --progress",
    "checking out the ref",
    "checkout ref",
];

const MIN_SHA_LEN: usize = 7;
const MAX_SHA_LEN: usize = 40;

/// The commit a runner actually checked out, read out of the runner's own log.
///
/// A workflow event's reported head SHA is metadata; this is evidence. They
/// disagree whenever a merge-queue or pull-request merge commit is what got
/// tested, so the two travel as separate fields and are never merged into one
/// `sha`.
pub struct CheckoutEvidence {
    pub commits: Vec<String>,
    pub lines: Vec<String>,
}

/// Split one `gh run view --log` line into its step column and its payload.
///
/// The line format is `job<TAB>step<TAB>timestamp payload`. The split matters:
/// the *step* column names a pinned action (`actions/checkout@<sha>`), and
/// harvesting that SHA would report the checkout action's own commit as the
/// commit under test — the exact conflation this scan exists to prevent.
fn split_log_line(line: &str) -> (&str, &str) {
    let line = line.trim_start_matches('\u{feff}');
    let mut columns = line.splitn(3, '\t');
    let (step, rest) = match (columns.next(), columns.next(), columns.next()) {
        (Some(_job), Some(step), Some(rest)) => (step, rest),
        _ => ("", line),
    };
    // Drop the leading ISO-8601 runner timestamp when there is one.
    let payload = match rest.split_once(' ') {
        Some((first, tail)) if first.contains('T') && first.ends_with('Z') => tail,
        _ => rest,
    };
    (step, payload)
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Whether a payload is a bare commit SHA on its own line.
///
/// `actions/checkout` prints the resolved commit this way, with no surrounding
/// prose, which is often the only place a branch-mode checkout records what it
/// actually landed on.
fn is_bare_commit_sha(payload: &str) -> bool {
    let trimmed = payload.trim();
    trimmed.len() == MAX_SHA_LEN && trimmed.bytes().all(is_hex)
}

/// Collect every lowercase-hex token of commit-SHA length in `payload`.
///
/// A token preceded by `@` is skipped: that is the `owner/action@sha` pin
/// form, which identifies the action, not the commit under test.
fn commit_sha_tokens(payload: &str) -> Vec<&str> {
    let bytes = payload.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphanumeric() {
            index += 1;
        }
        let token = &payload[start..index];
        let pinned = start > 0 && bytes[start - 1] == b'@';
        if !pinned
            && (MIN_SHA_LEN..=MAX_SHA_LEN).contains(&token.len())
            && token.bytes().all(is_hex)
        {
            tokens.push(token);
        }
    }
    tokens
}

/// Scan a runner log for checkout evidence, keeping at most `max_lines` lines.
pub fn scan_checkout_evidence(log: &str, max_lines: usize) -> CheckoutEvidence {
    let mut commits: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    for line in log.lines() {
        let (step, payload) = split_log_line(line);
        let lowered = payload.to_ascii_lowercase();
        let marked = CHECKOUT_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker));
        let in_checkout_step = step.to_ascii_lowercase().contains("checkout");
        let is_evidence = marked || (in_checkout_step && is_bare_commit_sha(payload));
        if !is_evidence {
            continue;
        }
        for token in commit_sha_tokens(payload) {
            if !commits.iter().any(|sha| sha == token) {
                commits.push(token.to_string());
            }
        }
        if lines.len() < max_lines {
            lines.push(redact_all(payload.trim()));
        }
    }
    CheckoutEvidence { commits, lines }
}

#[cfg(test)]
mod tests;

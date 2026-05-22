use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use colored::{ColoredString, Colorize};
use orbit_core::OrbitError;
use serde::Serialize;
use serde_json::Value;

use crate::parse::parse_since;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[clap(rename_all = "lower")]
pub enum LevelFilter {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LevelFilter {
    fn rank(self) -> u8 {
        match self {
            LevelFilter::Trace => 0,
            LevelFilter::Debug => 1,
            LevelFilter::Info => 2,
            LevelFilter::Warn => 3,
            LevelFilter::Error => 4,
        }
    }

    pub(crate) fn from_event_level(level: &str) -> Option<LevelFilter> {
        match level.to_ascii_uppercase().as_str() {
            "TRACE" => Some(LevelFilter::Trace),
            "DEBUG" => Some(LevelFilter::Debug),
            "INFO" => Some(LevelFilter::Info),
            "WARN" => Some(LevelFilter::Warn),
            "ERROR" => Some(LevelFilter::Error),
            _ => None,
        }
    }

    pub(crate) fn parse_query(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "trace" => Ok(LevelFilter::Trace),
            "debug" => Ok(LevelFilter::Debug),
            "info" => Ok(LevelFilter::Info),
            "warn" | "warning" => Ok(LevelFilter::Warn),
            "error" | "err" => Ok(LevelFilter::Error),
            other => Err(format!(
                "level must be one of trace, debug, info, warn, error; got '{other}'"
            )),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Filters {
    target_prefix: Option<String>,
    min_level: Option<LevelFilter>,
    since: Option<DateTime<Utc>>,
}

#[allow(dead_code)]
impl Filters {
    pub(crate) fn new(
        target_prefix: Option<String>,
        min_level: Option<LevelFilter>,
        since: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            target_prefix,
            min_level,
            since,
        }
    }

    pub(crate) fn from_query_parts(
        target: Option<String>,
        level: Option<String>,
        since: Option<&str>,
    ) -> Result<Self, OrbitError> {
        let min_level = match level.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(LevelFilter::parse_query(raw).map_err(OrbitError::InvalidInput)?),
            None => None,
        };
        let since = since.map(parse_since).transpose()?;
        Ok(Self::new(target, min_level, since))
    }

    pub(crate) fn matches(&self, event: &Value) -> bool {
        let target = event.get("target").and_then(Value::as_str).unwrap_or("");
        if let Some(prefix) = &self.target_prefix
            && !target.starts_with(prefix)
        {
            return false;
        }
        if let Some(min) = self.min_level {
            let level = event.get("level").and_then(Value::as_str).unwrap_or("INFO");
            let event_level = LevelFilter::from_event_level(level).unwrap_or(LevelFilter::Info);
            if event_level.rank() < min.rank() {
                return false;
            }
        }
        if let Some(since) = self.since
            && let Some(ts) = event.get("timestamp").and_then(Value::as_str)
            && let Ok(parsed) = DateTime::parse_from_rfc3339(ts)
            && parsed.with_timezone(&Utc) < since
        {
            return false;
        }
        true
    }
}

// Web-only types retained in the CLI copy after ORB-00146 extraction (the renderers
// live in orbit-dashboard now). Marked dead_code to keep the file diff minimal and
// avoid splitting the log module during this refactor.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RenderedLogEvent {
    pub ts: String,
    pub source: String,
    pub code: String,
    pub level: String,
    pub message_html: String,
}

#[allow(dead_code)]
pub(crate) fn build_filters(
    target: Option<String>,
    level: Option<LevelFilter>,
    since: Option<&str>,
) -> Result<Filters, OrbitError> {
    let since = since.map(parse_since).transpose()?;
    Ok(Filters::new(target, level, since))
}

#[allow(dead_code)]
pub(crate) fn resolve_log_path(override_path: Option<&Path>) -> Result<PathBuf, OrbitError> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    if let Ok(env) = std::env::var("ORBIT_LOG_PATH")
        && !env.is_empty()
    {
        return Ok(PathBuf::from(env));
    }
    orbit_common::utility::logging::global_jsonl_log_path().map_err(|err| {
        OrbitError::InvalidInput(format!("cannot resolve global JSONL log path: {err}"))
    })
}

#[allow(dead_code)]
pub(crate) fn read_recent_rendered_events(
    path: &Path,
    filters: &Filters,
    limit: usize,
) -> io::Result<Vec<RenderedLogEvent>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let reader = BufReader::new(file);
    let mut kept = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if let Some(event) = parse_matching_event(&line, filters) {
            kept.push(render_log_event_for_web(&event));
            if kept.len() > limit {
                kept.remove(0);
            }
        }
    }
    Ok(kept)
}

#[allow(dead_code)]
pub(crate) fn parse_matching_event(raw: &str, filters: &Filters) -> Option<Value> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    filters.matches(&value).then_some(value)
}

#[allow(dead_code)]
pub(crate) fn render_log_event_for_web(event: &Value) -> RenderedLogEvent {
    let ts = event
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let level_raw = event.get("level").and_then(Value::as_str).unwrap_or("INFO");
    let target = event.get("target").and_then(Value::as_str).unwrap_or("-");
    let fields = event
        .get("fields")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));

    RenderedLogEvent {
        ts,
        source: format_source(target, &fields),
        code: format_code(target, level_raw, &fields),
        level: normalize_level(level_raw).to_string(),
        message_html: format_message_html(target, &fields),
    }
}

pub(crate) fn format_event_line(event: &Value, use_color: bool) -> String {
    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or("--:--:--");
    let level = event.get("level").and_then(Value::as_str).unwrap_or("INFO");
    let target = event.get("target").and_then(Value::as_str).unwrap_or("-");
    let fields = event
        .get("fields")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));

    let time_col = format_timestamp(timestamp);
    let source_col = format_source(target, &fields);
    let code_col = format_code(target, level, &fields);
    let message_col = format_message(target, &fields);

    if use_color {
        format!(
            "{time}  {source:14}  {code}  {message}",
            time = time_col.dimmed(),
            source = colorize_source(target, &source_col),
            code = colorize_code(target, level, &code_col),
            message = message_col,
        )
    } else {
        format!("{time_col}  {source_col:14}  {code_col:5}  {message_col}")
    }
}

#[allow(dead_code)]
fn normalize_level(level: &str) -> &'static str {
    match level.to_ascii_uppercase().as_str() {
        "TRACE" => "trace",
        "DEBUG" => "debug",
        "WARN" => "warn",
        "ERROR" => "error",
        _ => "info",
    }
}

fn format_timestamp(raw: &str) -> String {
    // Accept ISO-8601 (RFC3339); display as HH:MM:SS in local-ish UTC. If the
    // string doesn't parse, render its first 8 chars after stripping the date.
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return parsed.with_timezone(&Utc).format("%H:%M:%S").to_string();
    }
    if let Some(idx) = raw.find('T') {
        let after_t = &raw[idx + 1..];
        return after_t.chars().take(8).collect();
    }
    raw.chars().take(8).collect()
}

pub(crate) fn format_source(target: &str, fields: &Value) -> String {
    // High-value targets get short, fixed labels for the source column.
    if let Some(label) = match target {
        "orbit.policy.deny" => Some("policy"),
        "orbit.friction.reported" => Some("friction"),
        t if t.starts_with("orbit.job.") => Some("job"),
        _ => None,
    } {
        return label.to_string();
    }

    // cli_runner subprocess events: prefer the `provider` field as the source
    // so the reader sees `claude-4.5` / `codex` / etc. directly.
    if target == "orbit_engine::activity_job::cli_runner"
        && let Some(provider) = fields.get("provider").and_then(Value::as_str)
    {
        return provider.to_string();
    }

    // Generic fallback: tail of the dotted target.
    target
        .rsplit_once('.')
        .map(|(_, tail)| tail.to_string())
        .unwrap_or_else(|| target.to_string())
}

pub(crate) fn format_code(target: &str, level: &str, fields: &Value) -> String {
    match target {
        "orbit.policy.deny" => "DENY".to_string(),
        "orbit.friction.reported" => "FRC".to_string(),
        "orbit.job.step_retry" => "RTRY".to_string(),
        "orbit.job.step_finished" => match fields.get("success").and_then(Value::as_bool) {
            Some(true) => "OK".to_string(),
            Some(false) => "ERR".to_string(),
            None => "INF".to_string(),
        },
        _ => match level {
            "ERROR" => "ERR".to_string(),
            "WARN" => "WRN".to_string(),
            "INFO" => "INF".to_string(),
            "DEBUG" => "DBG".to_string(),
            "TRACE" => "TRC".to_string(),
            other => other.chars().take(3).collect::<String>().to_uppercase(),
        },
    }
}

pub(crate) fn format_message(target: &str, fields: &Value) -> String {
    let getf = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("");
    let getn = |k: &str| -> String {
        fields
            .get(k)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };

    match target {
        "orbit.policy.deny" => {
            let tool = getf("tool");
            let path = getf("path");
            let profile = getf("profile");
            let rule = getf("matched_rule");
            let mut s = String::new();
            if !tool.is_empty() {
                s.push_str(&format!("tool={tool}"));
            }
            if !path.is_empty() {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&format!("path={path}"));
            }
            if !profile.is_empty() {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&format!("profile={profile}"));
            }
            if !rule.is_empty() {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&format!("rule={rule}"));
            }
            s
        }
        "orbit.friction.reported" => {
            let task_id = getf("task_id");
            let agent = getf("agent");
            let model = getf("model");
            let summary = getf("summary");
            let mut s = format!("friction reported on {task_id}");
            if !agent.is_empty() || !model.is_empty() {
                s.push_str(&format!(" by {agent}/{model}"));
            }
            if !summary.is_empty() {
                s.push_str(&format!(": {summary}"));
            }
            s
        }
        "orbit.job.step_started" => {
            format!(
                "step {} started [run={}]",
                getf("step_id"),
                getf("job_run_id"),
            )
        }
        "orbit.job.step_finished" => {
            let step = getf("step_id");
            let outcome = getf("outcome");
            let success = fields.get("success").and_then(Value::as_bool);
            match success {
                Some(true) => format!("step {step} finished ok ({outcome})"),
                Some(false) => format!("step {step} finished {outcome}"),
                None => format!("step {step} finished {outcome}"),
            }
        }
        "orbit.job.step_retry" => format!(
            "step {} retry attempt={} backoff_ms={}",
            getf("step_id"),
            getn("attempt"),
            getn("next_backoff_ms"),
        ),
        "orbit.job.step_skipped" => {
            format!("step {} skipped: {}", getf("step_id"), getf("reason"))
        }
        "orbit.job.step_denied" => {
            format!("step {} denied: {}", getf("step_id"), getf("reason"))
        }
        "orbit.job.fanout" => format!(
            "fanout phase={} step={} workers={} collected={} failed={}",
            getf("phase"),
            getf("step_id"),
            getn("worker_count"),
            getn("collected"),
            getn("failed"),
        ),
        "orbit.job.worker_state" => format!(
            "worker[{}] state={} step={}",
            getn("worker_index"),
            getf("state"),
            getf("step_id"),
        ),
        "orbit.job.loop_iteration" => format!(
            "loop {} phase={} step={}",
            getn("iteration"),
            getf("phase"),
            getf("step_id"),
        ),
        "orbit.job.loop_did_not_converge" => format!(
            "loop step={} did not converge after {} iterations",
            getf("step_id"),
            getn("max_iterations"),
        ),
        "orbit_engine::activity_job::cli_runner" => {
            let stream = getf("stream");
            let line = getf("line");
            if !stream.is_empty() {
                format!("[{stream}] {line}")
            } else {
                line.to_string()
            }
        }
        _ => {
            // Generic fallback: render fields as `key=value` space-separated,
            // omitting `message` (already handled above for known targets) and
            // `target` (already in the source column).
            let mut parts: Vec<String> = Vec::new();
            if let Value::Object(map) = fields {
                if let Some(message) = map.get("message").and_then(Value::as_str) {
                    parts.push(message.to_string());
                }
                for (k, v) in map {
                    if k == "message" {
                        continue;
                    }
                    let value_str = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    parts.push(format!("{k}={value_str}"));
                }
            }
            parts.join(" ")
        }
    }
}

// Retained for parity after ORB-00146 (web dashboard + its HTML renderers moved to
// orbit-dashboard). The non-HTML formatters are still used by `orbit log` etc.
#[allow(dead_code)]
pub(crate) fn format_message_html(target: &str, fields: &Value) -> String {
    let getf = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("");
    let getn = |k: &str| -> String {
        fields
            .get(k)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };

    match target {
        "orbit.policy.deny" => html_pairs(&[
            ("tool", getf("tool").to_string()),
            ("path", getf("path").to_string()),
            ("profile", getf("profile").to_string()),
            ("rule", getf("matched_rule").to_string()),
        ]),
        "orbit.friction.reported" => {
            let mut s = format!(
                "friction reported on {}",
                code_value(getf("task_id").to_string())
            );
            let agent = getf("agent");
            let model = getf("model");
            if !agent.is_empty() || !model.is_empty() {
                s.push_str(" by ");
                s.push_str(&code_value(format!("{agent}/{model}")));
            }
            let summary = getf("summary");
            if !summary.is_empty() {
                s.push_str(": ");
                s.push_str(&escape_html(summary));
            }
            s
        }
        "orbit.job.step_started" => format!(
            "step {} started [run={}]",
            code_value(getf("step_id").to_string()),
            code_value(getf("job_run_id").to_string()),
        ),
        "orbit.job.step_finished" => {
            let step = code_value(getf("step_id").to_string());
            let outcome = code_value(getf("outcome").to_string());
            match fields.get("success").and_then(Value::as_bool) {
                Some(true) => format!("step {step} finished ok ({outcome})"),
                Some(false) | None => format!("step {step} finished {outcome}"),
            }
        }
        "orbit.job.step_retry" => format!(
            "step {} retry attempt={} backoff_ms={}",
            code_value(getf("step_id").to_string()),
            code_value(getn("attempt")),
            code_value(getn("next_backoff_ms")),
        ),
        "orbit.job.step_skipped" => {
            format!(
                "step {} skipped: {}",
                code_value(getf("step_id").to_string()),
                escape_html(getf("reason")),
            )
        }
        "orbit.job.step_denied" => {
            format!(
                "step {} denied: {}",
                code_value(getf("step_id").to_string()),
                escape_html(getf("reason")),
            )
        }
        "orbit.job.fanout" => html_pairs(&[
            ("phase", getf("phase").to_string()),
            ("step", getf("step_id").to_string()),
            ("workers", getn("worker_count")),
            ("collected", getn("collected")),
            ("failed", getn("failed")),
        ]),
        "orbit.job.worker_state" => format!(
            "worker[{}] state={} step={}",
            code_value(getn("worker_index")),
            code_value(getf("state").to_string()),
            code_value(getf("step_id").to_string()),
        ),
        "orbit.job.loop_iteration" => format!(
            "loop {} phase={} step={}",
            code_value(getn("iteration")),
            code_value(getf("phase").to_string()),
            code_value(getf("step_id").to_string()),
        ),
        "orbit.job.loop_did_not_converge" => format!(
            "loop step={} did not converge after {} iterations",
            code_value(getf("step_id").to_string()),
            code_value(getn("max_iterations")),
        ),
        "orbit_engine::activity_job::cli_runner" => {
            let stream = getf("stream");
            let line = getf("line");
            if !stream.is_empty() {
                format!("[{}] {}", code_value(stream.to_string()), escape_html(line))
            } else {
                escape_html(line)
            }
        }
        _ => {
            let mut parts: Vec<String> = Vec::new();
            if let Value::Object(map) = fields {
                if let Some(message) = map.get("message").and_then(Value::as_str) {
                    parts.push(escape_html(message));
                }
                for (k, v) in map {
                    if k == "message" {
                        continue;
                    }
                    let value_str = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    parts.push(format!(
                        "<b>{}</b>={}",
                        escape_html(k),
                        code_value(value_str)
                    ));
                }
            }
            parts.join(" ")
        }
    }
}

#[allow(dead_code)]
fn html_pairs(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("<b>{}</b>={}", escape_html(key), code_value(value.clone())))
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(dead_code)]
fn code_value(value: String) -> String {
    format!("<code>{}</code>", escape_html(&value))
}

#[allow(dead_code)]
fn escape_html(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn colorize_source(target: &str, label: &str) -> ColoredString {
    match target {
        "orbit.policy.deny" => label.red().bold(),
        "orbit.friction.reported" => label.yellow(),
        t if t.starts_with("orbit.job.") => label.cyan(),
        "orbit_engine::activity_job::cli_runner" => label.magenta(),
        _ => label.normal(),
    }
}

fn colorize_code(target: &str, level: &str, code: &str) -> ColoredString {
    if target == "orbit.policy.deny" {
        return code.red().bold();
    }
    match level {
        "ERROR" => code.red().bold(),
        "WARN" => code.yellow(),
        "INFO" => code.green(),
        "DEBUG" => code.blue(),
        _ => code.normal(),
    }
}

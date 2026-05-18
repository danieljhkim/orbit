//! Process-log snapshot and SSE stream handlers.

use std::convert::Infallible;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use futures_core::Stream;
use tokio::sync::mpsc;

use super::{LogQuery, map_runtime_error, non_empty_string, server_error};
use crate::log_format::{
    Filters as LogFilters, RenderedLogEvent, parse_matching_event, read_recent_rendered_events,
    render_log_event_for_web, resolve_log_path,
};

const LOG_DEFAULT_LIMIT: usize = 50;
pub(super) const LOG_MAX_LIMIT: usize = 500;
const LOG_STREAM_CHANNEL_DEPTH: usize = 64;
const LOG_STREAM_POLL_INTERVAL: StdDuration = StdDuration::from_millis(50);

pub(super) async fn get_log(Query(q): Query<LogQuery>) -> Response {
    let path = match resolve_log_path(None) {
        Ok(path) => path,
        Err(e) => return map_runtime_error(e),
    };
    match read_log_snapshot_from_path(&path, &q) {
        Ok(events) => Json(events).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn stream_log(Query(q): Query<LogQuery>) -> Response {
    let path = match resolve_log_path(None) {
        Ok(path) => path,
        Err(e) => return map_runtime_error(e),
    };
    let filters = match log_filters(&q) {
        Ok(filters) => filters,
        Err(e) => return map_runtime_error(e),
    };
    let stream = ReceiverSseStream {
        rx: spawn_log_sse_frames(path, filters),
    };
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
    {
        Ok(response) => response,
        Err(e) => server_error(orbit_core::OrbitError::Execution(format!(
            "build SSE response: {e}"
        ))),
    }
}

fn read_log_snapshot_from_path(
    path: &std::path::Path,
    query: &LogQuery,
) -> Result<Vec<RenderedLogEvent>, orbit_core::OrbitError> {
    let limit = match query.limit {
        Some(limit) if limit > LOG_MAX_LIMIT => {
            return Err(orbit_core::OrbitError::InvalidInput(format!(
                "limit must be <= {LOG_MAX_LIMIT}; got {limit}"
            )));
        }
        Some(limit) => limit,
        None => LOG_DEFAULT_LIMIT,
    };
    let filters = log_filters(query)?;
    read_recent_rendered_events(path, &filters, limit)
        .map_err(|e| orbit_core::OrbitError::Io(format!("read log {}: {e}", path.display())))
}

fn log_filters(query: &LogQuery) -> Result<LogFilters, orbit_core::OrbitError> {
    LogFilters::from_query_parts(
        query.target.as_deref().and_then(non_empty_string),
        query.level.as_deref().and_then(non_empty_string),
        query
            .since
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )
}

fn spawn_log_sse_frames(path: PathBuf, filters: LogFilters) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(LOG_STREAM_CHANNEL_DEPTH);
    thread::spawn(move || {
        let mut offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut leftover = String::new();
        loop {
            if tx.is_closed() {
                return;
            }
            match read_appended_log_events(&path, &filters, &mut offset, &mut leftover) {
                Ok(events) => {
                    for event in events {
                        let frame = match format_sse_frame(&event) {
                            Ok(frame) => frame,
                            Err(_) => continue,
                        };
                        if tx.blocking_send(frame).is_err() {
                            return;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
            thread::sleep(LOG_STREAM_POLL_INTERVAL);
        }
    });
    rx
}

fn read_appended_log_events(
    path: &std::path::Path,
    filters: &LogFilters,
    offset: &mut u64,
    leftover: &mut String,
) -> io::Result<Vec<RenderedLogEvent>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < *offset {
        *offset = 0;
        leftover.clear();
    }
    file.seek(SeekFrom::Start(*offset))?;
    let mut reader = BufReader::new(file);
    let mut events = Vec::new();

    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            break;
        }
        *offset += n as u64;
        if !buf.ends_with('\n') {
            leftover.push_str(&buf);
            continue;
        }
        let mut full_line = String::new();
        if !leftover.is_empty() {
            full_line.push_str(leftover);
            leftover.clear();
        }
        full_line.push_str(buf.trim_end_matches('\n'));
        if let Some(event) = parse_matching_event(&full_line, filters) {
            events.push(render_log_event_for_web(&event));
        }
    }

    Ok(events)
}

fn format_sse_frame(event: &RenderedLogEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|json| format!("data: {json}\n\n"))
}

struct ReceiverSseStream {
    rx: mpsc::Receiver<String>,
}

impl Stream for ReceiverSseStream {
    type Item = Result<String, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;
    use tempfile::tempdir;

    use super::super::test_support::write_lines;
    use super::*;

    #[test]
    fn log_snapshot_filters_target_level_and_since() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(
            &path,
            &[
                json!({
                    "timestamp": "2026-04-27T01:00:01Z",
                    "level": "INFO",
                    "target": "orbit.policy.deny",
                    "fields": {"tool": "fs.read", "path": "/tmp/a"}
                })
                .to_string(),
                json!({
                    "timestamp": "2026-04-27T01:00:03Z",
                    "level": "WARN",
                    "target": "orbit.policy.deny",
                    "fields": {"tool": "fs.write", "path": "/etc/passwd"}
                })
                .to_string(),
                json!({
                    "timestamp": "2026-04-27T01:00:04Z",
                    "level": "ERROR",
                    "target": "orbit.job.step_finished",
                    "fields": {"step_id": "build", "outcome": "failed", "success": false}
                })
                .to_string(),
            ],
        );

        let events = read_log_snapshot_from_path(
            &path,
            &LogQuery {
                limit: Some(10),
                target: Some("orbit.policy".to_string()),
                level: Some("warn".to_string()),
                since: Some("2026-04-27T01:00:02Z".to_string()),
            },
        )
        .expect("snapshot");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "policy");
        assert_eq!(events[0].code, "DENY");
        assert_eq!(events[0].level, "warn");
        assert!(events[0].message_html.contains("<b>path</b>="));
    }

    #[test]
    fn log_snapshot_rejects_limit_above_max() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(&path, &[]);

        let err = read_log_snapshot_from_path(
            &path,
            &LogQuery {
                limit: Some(LOG_MAX_LIMIT + 1),
                ..LogQuery::default()
            },
        )
        .expect_err("limit should be rejected");

        assert!(err.to_string().contains("limit must be <= 500"));
    }

    #[test]
    fn log_stream_framing_emits_one_data_frame_per_appended_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(&path, &[]);
        let mut offset = std::fs::metadata(&path).expect("metadata").len();
        let mut leftover = String::new();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append");
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-04-27T01:00:05Z",
                "level": "INFO",
                "target": "orbit.job.step_started",
                "fields": {"job_run_id": "run-1", "step_id": "build"}
            })
        )
        .expect("write event");
        file.flush().expect("flush");

        let events =
            read_appended_log_events(&path, &LogFilters::default(), &mut offset, &mut leftover)
                .expect("read appended");
        assert_eq!(events.len(), 1);

        let frame = format_sse_frame(&events[0]).expect("frame");
        assert!(frame.starts_with("data: "));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"source\":\"job\""));
        assert!(frame.contains("build"));
    }
}

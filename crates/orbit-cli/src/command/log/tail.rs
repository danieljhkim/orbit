//! `orbit log tail` — column-formatted reader for the unified JSONL tracing
//! feed at `~/.orbit/state/logs/orbit.jsonl` (or wherever `--path` /
//! `ORBIT_LOG_PATH` points). Renders the v2-terminal-console mockup's four
//! columns: timestamp, source, code, message. Designed for human eyes by
//! default and pipeline-friendly when stdout is not a TTY (`--json` or
//! plain-text without ANSI escapes).

use std::fs::File;
use std::io::{self, BufRead, BufReader, IsTerminal, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;

use super::format::{
    Filters, LevelFilter, build_filters as build_shared_filters, format_event_line,
    resolve_log_path,
};

#[derive(Args)]
pub struct TailArgs {
    /// Number of recent lines to print before exiting (or before tailing in
    /// follow mode).
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,

    /// Tail the file as it grows. Stop on Ctrl-C.
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Filter by tracing target prefix (e.g. `--target orbit.policy`
    /// matches `orbit.policy.deny`).
    #[arg(long)]
    pub target: Option<String>,

    /// Filter by minimum log level. `error > warn > info > debug > trace`.
    #[arg(long)]
    pub level: Option<LevelFilter>,

    /// Filter by timestamp window (e.g. `5m`, `1h`, `30s`, RFC3339).
    #[arg(long)]
    pub since: Option<String>,

    /// Emit each event as one raw JSON line instead of the four-column view.
    #[arg(long)]
    pub json: bool,

    /// Override the JSONL path. Falls back to `$ORBIT_LOG_PATH`, then
    /// `$HOME/.orbit/state/logs/orbit.jsonl`. Provided primarily for tests.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

impl Execute for TailArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let path = resolve_log_path(self.path.as_deref())?;
        let filters = build_filters(&self)?;
        let stdout = io::stdout();
        let use_color = stdout.is_terminal();
        let mut writer = stdout.lock();
        run_tail(&path, &self, &filters, use_color, &mut writer).map_err(io_to_orbit)
    }
}

pub(super) fn build_filters(args: &TailArgs) -> Result<Filters, OrbitError> {
    build_shared_filters(args.target.clone(), args.level, args.since.as_deref())
}

pub(super) fn run_tail<W: Write>(
    path: &Path,
    args: &TailArgs,
    filters: &Filters,
    use_color: bool,
    writer: &mut W,
) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("orbit log file not found: {}", path.display()),
        ));
    }

    let initial_offset = print_initial_window(path, args, filters, use_color, writer)?;
    if !args.follow {
        return Ok(());
    }

    follow_file(path, initial_offset, filters, args.json, use_color, writer)
}

fn print_initial_window<W: Write>(
    path: &Path,
    args: &TailArgs,
    filters: &Filters,
    use_color: bool,
    writer: &mut W,
) -> io::Result<u64> {
    let file = File::open(path)?;
    let total_bytes = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut buf = String::new();
    let mut all = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            break;
        }
        all.push(buf.trim_end_matches('\n').to_string());
    }

    let kept: Vec<&String> = all
        .iter()
        .filter(|line| match serde_json::from_str::<Value>(line) {
            Ok(value) => filters.matches(&value),
            Err(_) => false,
        })
        .collect();

    let start = kept.len().saturating_sub(args.lines);
    for line in &kept[start..] {
        emit_line(line, args.json, use_color, writer)?;
    }
    Ok(total_bytes)
}

fn follow_file<W: Write>(
    path: &Path,
    initial_offset: u64,
    filters: &Filters,
    json: bool,
    use_color: bool,
    writer: &mut W,
) -> io::Result<()> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(initial_offset))?;
    let mut reader = BufReader::new(file);
    let mut leftover = String::new();

    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        if !buf.ends_with('\n') {
            // Partial line: stash and try again next iteration.
            leftover.push_str(&buf);
            continue;
        }
        let mut full_line = String::new();
        if !leftover.is_empty() {
            full_line.push_str(&leftover);
            leftover.clear();
        }
        full_line.push_str(buf.trim_end_matches('\n'));
        if let Ok(value) = serde_json::from_str::<Value>(&full_line)
            && filters.matches(&value)
        {
            emit_line(&full_line, json, use_color, writer)?;
        }
    }
}

fn emit_line<W: Write>(raw: &str, json: bool, use_color: bool, writer: &mut W) -> io::Result<()> {
    if json {
        writeln!(writer, "{raw}")?;
        return Ok(());
    }
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(v) => v,
        Err(_) => {
            // Skip malformed lines silently: the producer warns about cross-process
            // interleaves, and reader robustness is part of the JSONL contract.
            return Ok(());
        }
    };
    let formatted = format_event_line(&value, use_color);
    writeln!(writer, "{formatted}")
}

fn io_to_orbit(err: io::Error) -> OrbitError {
    OrbitError::InvalidInput(err.to_string())
}


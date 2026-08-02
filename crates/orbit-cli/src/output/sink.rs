//! The output sink: one answer per invocation to "who is reading this?".
//!
//! Resolved once at startup from stdout, the process environment, and the
//! global `--format` argument, per `docs/design/terminal-interface/specs/output-modes.md`
//! §1–§2 ([ADR-0306]). Nothing downstream re-derives these answers: the sink is
//! the only place in this crate that queries terminal state, apart from the
//! pre-existing local check in `command/log/tail.rs`. That property is enforced
//! by `scripts/check-terminal-state-guard.sh`.
//!
//! This is step 1 of the spec's §7 migration: the sink is *resolved* but not yet
//! *consumed*, so rendering is byte-for-byte unchanged. Later steps route the
//! per-command `--json` booleans through [`OutputSink::resolve`]'s `legacy_json`
//! rung, then let renderers read [`OutputSink::width`] and
//! [`OutputSink::color_allowed`].

use std::io::IsTerminal;

use clap::ValueEnum;

/// The value of the global `--format` argument, and of `ORBIT_FORMAT`.
///
/// `auto` is a request to decide from the sink; the other three name a
/// rendering directly. There is deliberately no `plain` variant — plain is a
/// rendering of `table` for a non-terminal sink, not a mode a caller can ask
/// for (spec §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    /// Decide from the sink: a table on a terminal, plain text otherwise.
    Auto,
    /// Aligned columns with a header.
    Table,
    /// A single JSON document.
    Json,
    /// One complete JSON document per line.
    Ndjson,
}

/// The mode a renderer renders in, after `auto` has been resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Aligned columns with a header, truncated to the sink's width.
    Table,
    /// `table` with the header suppressed, borders and ANSI absent, truncation
    /// disabled, and single-tab field separators — the form `cut -f` expects.
    Plain,
    /// A single JSON document.
    Json,
    /// One complete JSON document per line, flushed per record.
    Ndjson,
}

/// The environment variables the sink reads, captured once so that resolution
/// is a pure function of its inputs and can be tested without mutating the
/// process environment.
#[derive(Clone, Debug, Default)]
pub struct SinkEnv {
    /// `COLUMNS` — an explicit terminal width, preferred over querying stdout.
    pub columns: Option<String>,
    /// `ORBIT_FORMAT` — the environment rung of the mode precedence.
    pub format: Option<String>,
    /// `NO_COLOR` — any non-empty value disables color.
    pub no_color: Option<String>,
    /// `CLICOLOR_FORCE` — any non-empty value forces color on a terminal.
    pub clicolor_force: Option<String>,
    /// `TERM` — `dumb` disables color unconditionally.
    pub term: Option<String>,
}

impl SinkEnv {
    /// Capture the sink-relevant variables from the real process environment.
    pub fn from_process() -> Self {
        Self {
            columns: read_var("COLUMNS"),
            format: read_var("ORBIT_FORMAT"),
            no_color: read_var("NO_COLOR"),
            clicolor_force: read_var("CLICOLOR_FORCE"),
            term: read_var("TERM"),
        }
    }
}

fn read_var(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Everything a renderer is allowed to know about its destination, resolved
/// once per invocation.
#[derive(Clone, Copy, Debug)]
pub struct OutputSink {
    is_tty: bool,
    width: u16,
    color_allowed: bool,
    mode: OutputMode,
}

impl OutputSink {
    /// Resolve the sink for this invocation from the real process environment.
    ///
    /// Called exactly once, from `main`. `requested` is the global `--format`
    /// value, absent when the flag was not passed.
    pub fn from_process(requested: Option<FormatArg>) -> Self {
        let is_tty = std::io::stdout().is_terminal();
        // Only ask the terminal for its size when there is a terminal; a
        // non-TTY sink has width 0 no matter what the ioctl would report.
        let terminal_width = if is_tty { query_terminal_width() } else { None };
        Self::resolve(
            is_tty,
            &SinkEnv::from_process(),
            terminal_width,
            requested,
            // Migration step 2 threads the per-command `--json` boolean in
            // here. Step 1 has no access to it — the flag lives on 86
            // individual argument structs — so the legacy rung is inert and
            // every `--json` branch still decides for itself.
            false,
        )
    }

    /// Resolve a sink from explicit inputs.
    ///
    /// `terminal_width` is what querying the terminal reported, or `None` when
    /// there is nothing to query. `legacy_json` is the per-command `--json`
    /// boolean (mode precedence rung 2).
    ///
    /// Tests must construct sinks through here rather than through
    /// [`OutputSink::from_process`]: `make ci` runs without a TTY, so a test
    /// that relies on the ambient environment asserts the piped path while
    /// appearing to test the terminal path.
    pub fn resolve(
        is_tty: bool,
        env: &SinkEnv,
        terminal_width: Option<u16>,
        requested: Option<FormatArg>,
        legacy_json: bool,
    ) -> Self {
        Self {
            is_tty,
            width: resolve_width(is_tty, env, terminal_width),
            color_allowed: resolve_color(is_tty, env),
            mode: resolve_mode(is_tty, env, requested, legacy_json),
        }
    }

    /// Whether stdout is a terminal.
    pub fn is_tty(&self) -> bool {
        self.is_tty
    }

    /// The width to truncate to. `0` means "do not truncate" — never a
    /// guessed default, because assuming a width for a sink that has none
    /// silently truncates data in a file.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Whether ANSI styling may be emitted.
    pub fn color_allowed(&self) -> bool {
        self.color_allowed
    }

    /// The resolved output mode.
    pub fn mode(&self) -> OutputMode {
        self.mode
    }
}

/// `COLUMNS` first, then what the terminal reported, then 0.
///
/// A non-TTY sink is 0 regardless of either, so `COLUMNS=200 orbit … > file`
/// cannot width-adapt a file.
fn resolve_width(is_tty: bool, env: &SinkEnv, terminal_width: Option<u16>) -> u16 {
    if !is_tty {
        return 0;
    }
    env.columns
        .as_deref()
        .and_then(parse_width)
        .or(terminal_width)
        .unwrap_or(0)
}

/// A width is a positive integer; `0` and unparseable values mean "unknown"
/// and fall through to the next source rather than truncating to nothing.
fn parse_width(raw: &str) -> Option<u16> {
    match raw.trim().parse::<u16>() {
        Ok(0) | Err(_) => None,
        Ok(width) => Some(width),
    }
}

/// Color precedence, per `specs/color-and-styling.md` §2.
///
/// The non-TTY rung comes first and is absolute: a redirected stream is never
/// styled, whatever the environment claims. `--no-color` and `--color=always`
/// are rungs 1 and 3 of that list; they land with the flags themselves, which
/// this step does not introduce.
fn resolve_color(is_tty: bool, env: &SinkEnv) -> bool {
    if !is_tty {
        return false;
    }
    if env.term.as_deref() == Some("dumb") {
        return false;
    }
    if is_set(env.no_color.as_deref()) {
        return false;
    }
    if is_set(env.clicolor_force.as_deref()) {
        return true;
    }
    true
}

/// Mode precedence, per `specs/output-modes.md` §2. First match wins.
fn resolve_mode(
    is_tty: bool,
    env: &SinkEnv,
    requested: Option<FormatArg>,
    legacy_json: bool,
) -> OutputMode {
    // 1. `--format <mode>` explicitly passed.
    if let Some(format) = requested {
        return render_as(format, is_tty);
    }
    // 2. `--json` (per-command legacy alias).
    if legacy_json {
        return OutputMode::Json;
    }
    // 3. `ORBIT_FORMAT`. An unrecognized value falls through to `auto` rather
    //    than failing the command — an exported variable must not break every
    //    invocation in the shell that set it.
    if let Some(format) = env
        .format
        .as_deref()
        .and_then(|raw| FormatArg::from_str(raw.trim(), true).ok())
    {
        return render_as(format, is_tty);
    }
    // 4. `auto`.
    render_as(FormatArg::Auto, is_tty)
}

/// Resolve `auto` against the sink; the other modes name their rendering.
fn render_as(format: FormatArg, is_tty: bool) -> OutputMode {
    match format {
        FormatArg::Auto if is_tty => OutputMode::Table,
        FormatArg::Auto => OutputMode::Plain,
        FormatArg::Table => OutputMode::Table,
        FormatArg::Json => OutputMode::Json,
        FormatArg::Ndjson => OutputMode::Ndjson,
    }
}

fn is_set(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Ask the terminal attached to stdout for its column count.
#[cfg(unix)]
fn query_terminal_width() -> Option<u16> {
    // SAFETY: `winsize` is a plain-data struct with no invalid bit patterns,
    // and `TIOCGWINSZ` only writes into it. The return code is checked before
    // the struct is read.
    let size = unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        let rc = libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &raw mut size);
        if rc != 0 {
            return None;
        }
        size
    };
    (size.ws_col > 0).then_some(size.ws_col)
}

/// No terminal query outside unix; `COLUMNS` is the only width source there.
#[cfg(not(unix))]
fn query_terminal_width() -> Option<u16> {
    None
}

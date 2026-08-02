//! Sink resolution tests.
//!
//! Every sink here is built with [`OutputSink::resolve`], never
//! [`OutputSink::from_process`]: `make ci` runs without a TTY, so a test that
//! read the ambient environment would assert the piped path while appearing to
//! test the terminal path.

use crate::output::sink::{FormatArg, OutputMode, OutputSink, SinkEnv};

/// A `SinkEnv` with every variable unset.
fn empty_env() -> SinkEnv {
    SinkEnv::default()
}

fn sink(is_tty: bool, env: &SinkEnv, terminal_width: Option<u16>) -> OutputSink {
    OutputSink::resolve(is_tty, env, terminal_width, None, false)
}

// ── §1 The sink's properties ────────────────────────────────────────────────

#[test]
fn non_tty_sink_has_zero_width_and_no_color_however_loudly_the_env_claims_otherwise() {
    let env = SinkEnv {
        columns: Some("200".to_string()),
        clicolor_force: Some("1".to_string()),
        ..empty_env()
    };

    let sink = sink(false, &env, Some(120));

    assert!(!sink.is_tty());
    assert_eq!(
        sink.width(),
        0,
        "a redirected stream is never width-adapted, even with COLUMNS set"
    );
    assert!(
        !sink.color_allowed(),
        "a redirected stream is never styled, even with CLICOLOR_FORCE set"
    );
}

#[test]
fn absent_width_disables_truncation_rather_than_defaulting_to_eighty() {
    // A terminal that reports no size: no COLUMNS, no answer from the query.
    let sink = sink(true, &empty_env(), None);

    assert!(sink.is_tty());
    assert_eq!(
        sink.width(),
        0,
        "unknown width means do not truncate; guessing 80 silently cuts data"
    );
}

#[test]
fn columns_env_wins_over_the_terminal_query() {
    let env = SinkEnv {
        columns: Some("100".to_string()),
        ..empty_env()
    };

    assert_eq!(sink(true, &env, Some(120)).width(), 100);
}

#[test]
fn terminal_query_answers_when_columns_is_unusable() {
    for columns in ["", "0", "not-a-number", "-5"] {
        let env = SinkEnv {
            columns: Some(columns.to_string()),
            ..empty_env()
        };

        assert_eq!(
            sink(true, &env, Some(120)).width(),
            120,
            "COLUMNS={columns:?} is not a width and must fall through"
        );
    }
}

#[test]
fn tty_sink_allows_color_by_default() {
    assert!(sink(true, &empty_env(), Some(80)).color_allowed());
}

#[test]
fn no_color_disables_color_and_outranks_clicolor_force() {
    let env = SinkEnv {
        no_color: Some("1".to_string()),
        clicolor_force: Some("1".to_string()),
        ..empty_env()
    };

    assert!(!sink(true, &env, Some(80)).color_allowed());
}

#[test]
fn empty_no_color_does_not_disable_color() {
    let env = SinkEnv {
        no_color: Some(String::new()),
        ..empty_env()
    };

    assert!(sink(true, &env, Some(80)).color_allowed());
}

#[test]
fn dumb_terminal_disables_color_even_when_forced() {
    let env = SinkEnv {
        term: Some("dumb".to_string()),
        clicolor_force: Some("1".to_string()),
        ..empty_env()
    };

    assert!(!sink(true, &env, Some(80)).color_allowed());
}

// ── §2 Mode resolution, one test per precedence rung ────────────────────────

#[test]
fn rung_one_explicit_format_outranks_every_lower_rung() {
    let env = SinkEnv {
        format: Some("ndjson".to_string()),
        ..empty_env()
    };

    let sink = OutputSink::resolve(true, &env, Some(80), Some(FormatArg::Table), true);

    assert_eq!(sink.mode(), OutputMode::Table);
}

#[test]
fn rung_one_explicit_auto_is_still_an_explicit_choice() {
    // `--format auto` was passed, so the legacy `--json` rung is not reached.
    let sink = OutputSink::resolve(true, &empty_env(), Some(80), Some(FormatArg::Auto), true);

    assert_eq!(sink.mode(), OutputMode::Table);
}

#[test]
fn rung_two_legacy_json_outranks_the_environment() {
    let env = SinkEnv {
        format: Some("table".to_string()),
        ..empty_env()
    };

    let sink = OutputSink::resolve(false, &env, None, None, true);

    assert_eq!(sink.mode(), OutputMode::Json);
}

#[test]
fn rung_three_environment_variable_selects_the_mode() {
    for (raw, expected) in [
        ("json", OutputMode::Json),
        ("ndjson", OutputMode::Ndjson),
        ("table", OutputMode::Table),
        // Case-insensitive and whitespace-tolerant: an exported variable is
        // typed by hand.
        (" NDJSON ", OutputMode::Ndjson),
    ] {
        let env = SinkEnv {
            format: Some(raw.to_string()),
            ..empty_env()
        };

        assert_eq!(
            OutputSink::resolve(false, &env, None, None, false).mode(),
            expected,
            "ORBIT_FORMAT={raw:?}"
        );
    }
}

#[test]
fn rung_three_environment_auto_resolves_against_the_sink() {
    let env = SinkEnv {
        format: Some("auto".to_string()),
        ..empty_env()
    };

    assert_eq!(
        OutputSink::resolve(true, &env, Some(80), None, false).mode(),
        OutputMode::Table
    );
    assert_eq!(
        OutputSink::resolve(false, &env, None, None, false).mode(),
        OutputMode::Plain
    );
}

#[test]
fn unrecognized_environment_value_falls_through_to_auto() {
    let env = SinkEnv {
        format: Some("yaml".to_string()),
        ..empty_env()
    };

    assert_eq!(
        OutputSink::resolve(true, &env, Some(80), None, false).mode(),
        OutputMode::Table,
        "a stray ORBIT_FORMAT must not break every command in that shell"
    );
}

#[test]
fn rung_four_auto_is_table_on_a_terminal_and_plain_off_one() {
    assert_eq!(sink(true, &empty_env(), Some(80)).mode(), OutputMode::Table);
    assert_eq!(sink(false, &empty_env(), None).mode(), OutputMode::Plain);
}

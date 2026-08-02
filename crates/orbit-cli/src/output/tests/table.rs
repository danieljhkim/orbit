//! Rendering assertions for the borderless table contract (ADR-0307).
//!
//! Every case pins the width explicitly rather than reading `COLUMNS`, so the
//! geometry is the same under `cargo test` and under `--nocapture`.

use crate::output::table::{Column, Table, build_table};

const BOX_GLYPHS: &[char] = &['─', '│', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '+'];

/// A result set with values long enough to force truncation at any sane width.
fn tool_list() -> Table {
    let mut table = Table::new(vec![
        Column::new("NAME").fixed(),
        Column::new("STATUS").fixed(),
        Column::new("BUILTIN").fixed(),
        Column::new("DESCRIPTION"),
    ]);
    table.add_row(vec![
        "orbit.task.locks.reserve",
        "inactive",
        "yes",
        "Reserve context-file locks for a task. Exactly one of `task_ids` or `files` must be given.",
    ]);
    table.add_row(vec![
        "orbit.task.show",
        "active",
        "yes",
        "Show one task with its comments, history, artifacts, and optionally its related docs.",
    ]);
    table.add_row(vec![
        "orbit.docs.index",
        "inactive",
        "yes",
        "Reindex the human-authored docs corpus so semantic search sees the current files.",
    ]);
    table
}

fn lines(rendered: &str) -> Vec<&str> {
    rendered.lines().collect()
}

#[test]
fn renders_one_line_per_record_with_a_header_and_no_box_glyphs() {
    let table = tool_list();

    for width in [None, Some(200), Some(120), Some(80), Some(40)] {
        let rendered = table.render(width, false);
        let lines = lines(&rendered.body);
        assert_eq!(
            lines.len(),
            4,
            "3 records plus one header at width {width:?}: {}",
            rendered.body
        );
        assert!(
            !rendered.body.chars().any(|c| BOX_GLYPHS.contains(&c)),
            "no box drawing at width {width:?}: {}",
            rendered.body
        );
    }
}

#[test]
fn columns_are_two_spaces_apart_with_no_leading_indent() {
    let mut table = build_table(&["ID", "TITLE"]);
    table.add_row(vec!["T1", "first"]);
    table.add_row(vec!["T2", "second"]);

    let rendered = table.render(None, false);

    assert_eq!(rendered.body.lines().next(), Some("ID  TITLE"));
    assert_eq!(rendered.body.lines().nth(1), Some("T1  first"));
    // Trailing padding is trimmed, so `cut`/`grep` see no phantom field.
    assert_eq!(rendered.body.lines().nth(2), Some("T2  second"));
}

#[test]
fn overflow_is_truncated_with_a_single_ellipsis() {
    let rendered = tool_list().render(Some(80), false);

    assert!(
        rendered.body.contains('…'),
        "long descriptions must truncate at 80 columns: {}",
        rendered.body
    );
    for line in lines(&rendered.body) {
        assert!(
            line.chars().filter(|c| *c == '…').count() <= 1,
            "one ellipsis per line: {line}"
        );
        assert!(
            line.chars().count() <= 80,
            "line exceeds the pinned width: {line}"
        );
    }
}

#[test]
fn fixed_columns_keep_their_width_while_flexible_ones_shrink() {
    let table = tool_list();

    let wide = table.render(Some(200), false);
    let narrow = table.render(Some(60), false);

    for rendered in [&wide, &narrow] {
        assert!(
            rendered.body.contains("orbit.task.locks.reserve"),
            "a fixed identifier column is never squeezed: {}",
            rendered.body
        );
    }
    assert!(
        !narrow
            .body
            .contains("Reserve context-file locks for a task."),
        "the flexible description column absorbs the squeeze: {}",
        narrow.body
    );
}

#[test]
fn flexible_columns_drop_from_the_right_and_say_so_on_stderr() {
    // The fixed columns plus a floored DESCRIPTION need 44 columns, so the
    // flexible one is dropped rather than squeezed below its floor.
    let rendered = tool_list().render(Some(40), false);

    assert!(
        !rendered.body.contains("DESCRIPTION"),
        "the rightmost flexible column is dropped: {}",
        rendered.body
    );
    assert_eq!(
        rendered.notices,
        vec!["columns hidden to fit the terminal: DESCRIPTION".to_string()],
        "a dropped column is reported, on stderr, never in the body"
    );
    assert_eq!(lines(&rendered.body).len(), 4, "records are not dropped");
}

#[test]
fn a_column_with_one_value_across_every_row_is_suppressed() {
    let rendered = tool_list().render(None, false);

    assert!(
        !rendered.body.contains("BUILTIN"),
        "every tool is builtin, so the column carries nothing: {}",
        rendered.body
    );
    assert!(
        rendered.body.contains("STATUS"),
        "status varies across the result set and stays: {}",
        rendered.body
    );
}

#[test]
fn a_suppressed_column_survives_when_the_caller_filtered_on_it() {
    let mut table = Table::new(vec![
        Column::new("ID").fixed(),
        Column::new("STATUS").fixed().filtered(true),
    ]);
    table.add_row(vec!["T1", "done"]);
    table.add_row(vec!["T2", "done"]);

    let rendered = table.render(None, false);

    assert!(
        rendered.body.starts_with("ID  STATUS"),
        "`--status done` keeps STATUS on screen: {}",
        rendered.body
    );
}

#[test]
fn keep_all_columns_opts_a_fixed_shape_view_out_of_suppression() {
    let mut table = build_table(&["COMPONENT", "CURRENT"]).keep_all_columns();
    table.add_row(vec!["workspace layout", "3"]);
    table.add_row(vec!["store schema", "3"]);

    let rendered = table.render(None, false);

    assert!(
        rendered.body.contains("CURRENT"),
        "a repeated version column still names itself: {}",
        rendered.body
    );
    assert_eq!(lines(&rendered.body).len(), 3);
}

#[test]
fn a_single_record_keeps_every_column() {
    let mut table = build_table(&["ID", "STATUS"]);
    table.add_row(vec!["T1", "done"]);

    let rendered = table.render(None, false);

    assert_eq!(lines(&rendered.body), vec!["ID  STATUS", "T1  done"]);
}

#[test]
fn numeric_columns_are_right_aligned_under_a_unit_header() {
    let mut table = Table::new(vec![
        Column::new("#").number(),
        Column::new("STATE").fixed(),
        Column::new("DURATION (ms)").number(),
    ]);
    table.add_row(vec!["1", "success", "187"]);
    table.add_row(vec!["2", "failed", "42910"]);

    let rendered = table.render(None, false);

    assert_eq!(
        lines(&rendered.body),
        vec![
            "#  STATE    DURATION (ms)",
            "1  success            187",
            "2  failed           42910",
        ],
        "magnitudes line up on their right edge: {}",
        rendered.body
    );
}

#[test]
fn a_path_loses_its_middle_rather_than_its_identifying_tail() {
    let mut table = Table::new(vec![
        Column::new("PATH").path(),
        Column::new("TYPE").fixed(),
    ]);
    table.add_row(vec![
        "docs/design/terminal-interface/specs/table-rendering.md",
        "design",
    ]);
    table.add_row(vec!["docs/design/terminal-interface/1_overview.md", "spec"]);

    let rendered = table.render(Some(40), false);

    assert!(
        rendered.body.contains("le-rendering.md") && rendered.body.contains("1_overview.md"),
        "the identifying tail survives: {}",
        rendered.body
    );
    assert!(
        rendered.body.contains("docs/design/term…"),
        "the head survives and the middle is elided: {}",
        rendered.body
    );
}

#[test]
fn a_multi_line_value_still_occupies_exactly_one_line() {
    let mut table = build_table(&["ID", "SUMMARY"]);
    table.add_row(vec!["T1", "first line\nsecond line\nthird line"]);
    table.add_row(vec!["T2", "single line"]);

    let rendered = table.render(None, false);

    assert_eq!(lines(&rendered.body).len(), 3);
    assert!(
        !rendered.body.contains("second line"),
        "the wrapped remainder is truncated, not printed: {}",
        rendered.body
    );
}

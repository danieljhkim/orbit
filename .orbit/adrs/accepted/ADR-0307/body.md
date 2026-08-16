## Context

`crates/orbit-cli/src/output/table.rs` builds every list view (21 call sites) with `presets::UTF8_BORDERS_ONLY` and `ContentArrangement::DynamicFullWidth`. Two properties follow from that pairing.

The preset draws box rules around and between the header and body. Those glyphs carry no information that column alignment does not already carry, and they are noise to `grep`, `cut`, and `awk` — the tools an operator reaches for when a list is longer than a screen.

`DynamicFullWidth` expands the table to the full terminal width and wraps overflowing cells rather than truncating them. `build_table` sets a truncation indicator, but truncation only applies to rows added through `add_single_line_row`, which caps `max_height(1)`. Only 2 of the 21 call sites use it. So in practice a long description or a multi-parameter input summary wraps to three or four lines, and a row is no longer a line. That is the actual reason the borders look necessary: once rows span variable numbers of lines, a horizontal rule is the only thing separating them. The border is compensating for the wrapping.

## Decision

Tables are borderless, and a row is exactly one line.

- Drop the box preset. A list is a dim uppercase header row, two-space gutters, and left-aligned columns. Numeric and duration columns are right-aligned so magnitudes compare vertically.
- Every row is capped to one line. Content that does not fit its column is truncated with `…`, never wrapped. `add_single_line_row` becomes the only way to add a row; the unbounded `add_row` path is not exposed.
- Truncation is a promise of a fuller view. A list command that truncates a column must have a corresponding detail command that prints the untruncated value (`orbit tool show <name>` for `orbit tool list`), and `--format json` always carries the full value regardless of terminal width.
- Columns whose value is identical in every row of a given result set are not rendered in `auto` mode.

Rejected alternative: keep borders and fix only the wrapping. Rejected because once rows are single-line the borders have nothing left to do — they were separating wrapped rows — and they remain hostile to line-oriented tooling.

Rejected alternative: an `--wide` flag that restores wrapping for full values. Rejected as a second rendering mode to maintain when `--format json` already returns the complete payload and the detail command already exists.

## Consequences

- A row is a line, so `orbit tool list | grep github` returns whole records rather than fragments of wrapped cells.
- Vertical scanning improves: aligned columns without rules put more rows on a screen and remove the glyph noise between them.
- The suppression rule removes low-entropy columns automatically — `BUILTIN` is `yes` for every row of the current `orbit tool list`, and disappears without a per-command decision.
- Cost: information leaves the list view. A long description is now only fully visible via the detail command or `--format json`, so a list becomes a navigation surface rather than a complete one, and every list command must have a detail counterpart that currently may not exist.
- Cost: the rendering depends on terminal width, so the same command truncates differently in an 80-column and a 200-column terminal. Any comparison of two captured outputs must fix `COLUMNS` or use `--format json`.
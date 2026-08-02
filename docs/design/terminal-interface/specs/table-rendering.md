---
type: design
summary: "Spec: Table Rendering"
last_validated: 2026-08-01
---

# Spec: Table Rendering

A list rendered by `orbit` is a header row followed by one line per record. Borders are not drawn, cells are truncated rather than wrapped, and every truncated value is retrievable in full from a named alternative. A consumer may assume that line count minus one equals record count, and that `cut -f` on the plain form yields whole field values.

## Why This Exists

The current `build_table` pairs `UTF8_BORDERS_ONLY` with `ContentArrangement::DynamicFullWidth`, which wraps overflowing cells. A wrapped row occupies a variable number of lines, which breaks every line-oriented tool an operator would reach for and forces a horizontal rule to act as the row separator. The border is not decoration to be argued about on taste; it is compensation for the wrapping. Removing the wrapping is what makes removing the border safe. Rationale in [ADR-0307].

## 1. Structure

- **Header.** One row, uppercase, dim, no rule beneath it. Present in `table` mode; absent in the plain (piped) form so consumers need not skip a line. Suppress the header when the result set is empty — print the empty-state line from §6 instead.
- **Body.** One line per record. No leading indent, no outer border, no column separators.
- **Gutter.** Exactly two spaces between columns. Padding is spaces only; never tabs in `table` mode.
- **No footer.** Counts, totals, and pagination hints go to stderr or a `--stats` flag, never into the table body where a consumer would parse them as a record.

## 2. Column Widths

Widths are computed from the result set, never declared as literals.

1. Each column's natural width is the maximum display width of its header and its cells, measured in grapheme clusters, not bytes.
2. If the sum of natural widths plus gutters fits the sink width, use natural widths and stop.
3. Otherwise, shrink only columns marked *flexible*, largest first, until the total fits. Columns marked *fixed* (IDs, statuses, timestamps, durations) never shrink.
4. A flexible column has a floor of 8 display columns. If the total still does not fit with every flexible column at its floor, drop flexible columns from the right until it does, and note the dropped columns on stderr.

**Invariant:** the same result set rendered at two terminal widths differs only in truncation and column presence — never in row count, row order, or the value of a fixed column.

## 3. Alignment

- Text, IDs, names, paths: left.
- Counts, durations, sizes, percentages: **right**, so magnitudes compare vertically.
- A numeric column carries its unit in the header (`DURATION (ms)`), not repeated in every cell. `187` right-aligned under a unit header beats `187ms` left-aligned.
- Timestamps: left, fixed-width format, never localized. Use a single format across the CLI.

## 4. Truncation

- Overflow is truncated with a single `…` occupying one display column, never wrapped. `Row::max_height(1)` is not an option a call site may decline.
- Truncate the **middle** for values whose tail is identifying (paths, branch refs, hashes); truncate the **tail** for prose (descriptions, titles).
- **Truncation implies a detail path.** A list command that can truncate column *C* must have a documented command that prints *C* in full for one record. Adding a truncatable column without one is incomplete.
- Truncation never applies to `json`, `ndjson`, or the plain piped form. Those carry full values at any width.

**Failure mode this prevents:** silent loss. A value that is cut with no `…` and no way to recover it is indistinguishable from a value that was short.

## 5. Column Selection

- **Uniform-value suppression.** In `auto`/`table` mode, a column whose value is identical across every row of the result set is not rendered. It stays present in `json`. This is what removes `BUILTIN` from an all-builtin `orbit tool list` without a per-command decision.
- Suppression is computed per invocation, so a filtered result set may show fewer columns than an unfiltered one. That is intended: the column carried no information *for this result set*.
- `--format table` (explicit) disables suppression, so a caller who wants stable columns can ask for them.
- Never suppress a column the user filtered on — if `--status done` was passed, `STATUS` renders even though it is uniform.

## 6. Empty and Degenerate Results

- Zero records: print one line to **stderr** naming what was searched and, where cheap, why it may be empty (`no tasks matching --status done (12 tasks total)`). Print nothing to stdout. A consumer piping the command receives an empty stream, not prose.
- One record: still render the table. Do not switch to a key-value view — the shape must not depend on the count.
- A cell whose value is absent renders as `-` in `table` mode and `null` in `json`. Never an empty cell; an empty cell and a missing column are visually identical.

## 7. Migration

The path from current behavior:

1. Change the preset in `crates/orbit-cli/src/output/table.rs` to a borderless one and switch `ContentArrangement` off full-width wrapping.
2. Make `add_single_line_row` the only exported row constructor; convert the 19 call sites that use `Table::add_row` directly.
3. Move width computation behind the sink (see [./output-modes.md](./output-modes.md) §1) so it is not resolved by `comfy-table` from a terminal that may not exist.
4. Convert `print_audit_event_line` and the other hand-padded `println!` sites to the table path.
5. Add per-column *fixed*/*flexible* and alignment metadata at each call site.

Steps 1–2 are mechanical and independently landable. Step 3 depends on [ADR-0306]. Steps 4–5 are per-command and may proceed incrementally.

There is no output snapshot suite to update — see [../2_design.md §8](../2_design.md#8-test-coverage-of-output) — so the migration must add fixtures rather than adjust them. At minimum: one golden file per mode at a pinned width, and an assertion that the plain form of an *N*-record result has exactly *N* lines.

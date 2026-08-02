---
title: Terminal Interface — Design
owner: claude
last_updated: 2026-08-02
last_validated: 2026-08-02
status: Accepted
feature: terminal-interface
doc_role: design
type: design
summary: "Current orbit-cli output implementation — borderless single-line tables, one semantic color mapping behind legacy wrappers, sink resolved but unconsumed — and where it still diverges from the specs."
tags: [terminal-interface]
paths: ["crates/orbit-cli/src/output/**", "crates/orbit-cli/src/command/**"]
related_features: [terminal-interface]
related_artifacts: [ADR-0306, ADR-0307, ADR-0308]
---

# Terminal Interface — Design

This document describes what `orbit-cli` renders today. It is deliberately a record of the current implementation, not of the target: the target is prescribed by [./specs/](./specs/) and decided in [./4_decisions.md](./4_decisions.md). Where current behavior diverges from a spec, this doc says so and names the spec, so a reviewer can tell an intentional gap from a regression. Web dashboard rendering is out of scope and lives in [user-interface/2_design.md](../user-interface/2_design.md).

## 1. The Output Module

`crates/orbit-cli/src/output/` has three submodules and 601 lines total. `table.rs` owns list rendering, `color.rs` maps domain values to styles, `json.rs` serializes payloads and error envelopes. There is no renderer abstraction above them: they are helper functions that command bodies call directly. `table.rs` is the one module with a rendering policy of its own rather than a pass-through to a library (§2).

`json.rs` is the closest thing to a contract. It owns `error_payload`, which projects an `OrbitError` into `{error, code}` plus optional `did_you_mean`, `artifact_origin`, and task-bundle corruption fields. Error output is therefore already payload-shaped in a way that normal output is not. Two caveats: the `code` discriminator gained a `_ => "internal_error"` catch-all when `OrbitError` became `#[non_exhaustive]` [ORB-10356], so an unmapped variant degrades silently rather than failing to compile; and a `RemoteTool` error returns the remote payload verbatim, bypassing the shape entirely, so a consumer cannot assume `code` is present.

## 2. Table Construction

`output/table.rs` owns a `Table` type that buffers rows and renders them itself; `comfy_table` is an implementation detail it never hands out. `Table::add_row` is the only row constructor and caps every row at `Row::max_height(1)`, so the unbounded path that made a row span three or four lines is unreachable from a command module [ORB-10567]. The 21 command modules construct either `build_table(&headers)` (all columns plain text) or `Table::new(vec![Column…])` when a column is an identifier, a number, or a path.

Rendering applies the `NOTHING` preset with `ContentArrangement::Disabled`, two-space right padding on every column but the last, and a dim header row. Widths are computed from the result set: natural widths first, then flexible columns shrink widest-first to a floor of 8, then flexible columns drop from the right with a notice on stderr. Fixed columns never move. Overflow truncates with `…` — tail truncation is `comfy_table`'s, middle truncation (`Column::path`) is applied before the grid sees the cell.

Two selection rules run before layout: a column whose value is identical in every row of a result set is suppressed unless the caller marked it `filtered` (the user filtered on it) or the view opted out with `keep_all_columns`, and a result set with no records prints its empty-state line to stderr, leaving stdout empty. Satisfies [./specs/table-rendering.md](./specs/table-rendering.md) §§1–6 [ADR-0307]. **Still diverges on §1's plain form** — the header renders in the piped form too, because mode resolution has not landed [ADR-0306] — and on width *sourcing*: `sink_width` reads `COLUMNS` and the terminal directly rather than an output sink. [./references/detail-commands.md](./references/detail-commands.md) records which truncatable column has a detail command and which four views still lack one.

## 3. Hand-Padded Line Output

Not every list goes through `build_table`. `orbit audit list` prints each event through `print_audit_event_line` in `command/audit/support.rs`, a single `println!` with the format string `"[{}] {:<8} {:<6} {}:{:<20} {}ms"`.

The column widths are literals, not functions of the result set, so alignment holds only while every value fits — a tool name longer than 20 characters shifts the duration column for that row alone. There is no header, so the columns are unlabeled. The duration is left-aligned with a trailing unit, so `187ms` and `0ms` do not align on their magnitudes. **Diverges from [./specs/table-rendering.md](./specs/table-rendering.md) §2 and §3** (computed widths, right-aligned numerics).

98 modules under `command/` call `println!` directly for some part of their output, so this is a pattern rather than an isolated case — a consequence of the flattened per-command layout [ORB-00279], where each module owns its rendering end to end.

This file is also where the drift between the two renderings is easiest to see. [ORB-10228] added trusted session-context fields (`workspace_id`, `caller_machine_id`, `transport`, `mcp_call_id`) to `audit_event_to_json` in the same file, and left `print_audit_event_line` untouched. The JSON view gained provenance the human view has never shown, and nothing flagged the asymmetry.

## 4. Two Color Vocabularies

`color.rs` defines the same mappings twice in incompatible types. Five functions — `status_color_cell`, `priority_color_cell`, `job_state_color_cell`, `doctor_status_color_cell`, `task_type_color_cell` — return `comfy_table::Cell` values for table rows. Three — `status_color`, `priority_color`, `job_state_color` — return `String` values styled by the `colored` crate for line output. The backend is chosen by the call site's rendering shape, not by the value's meaning. `doctor_status_color_cell` and `task_type_color_cell` have no String counterpart, so the pairing is not even consistent; `task_type_color_cell` applies no styling at all and exists only to satisfy the cell-returning shape.

The copies have already drifted: `status_color` maps `backlog` explicitly, `status_color_cell` does not. The two-edit cost is visible in the history — [T20260427-43] added a `friction` arm to both functions, and [ORB-10202] removed it from both when the status was retired. The palette itself is broader than a closed role set — `in-progress` is cyan, `review` is magenta, `done` is bold green — so it encodes more distinctions than the target vocabulary. **Diverges from [./specs/color-and-styling.md](./specs/color-and-styling.md)** [ADR-0308].

## 5. Sink Resolution Without Consumers

`output/sink.rs` resolves `is_tty`, `width`, `color_allowed`, and the output mode once per invocation, from `main.rs`, ahead of dispatch [ORB-10569]. It is the only place in the crate that queries terminal state — `COLUMNS`, `TIOCGWINSZ`, `NO_COLOR`, `CLICOLOR_FORCE`, `IsTerminal` — enforced by `scripts/check-terminal-state-guard.sh`, whose allowlist carries the one grandfathered exception: `command/log/tail.rs` still calls `stdout.is_terminal()` locally to decide whether to colorize the tailed stream.

**Nothing consumes the answers yet.** That is step 1 of [./specs/output-modes.md](./specs/output-modes.md) §7, which deliberately changes no rendering: the sink is resolved and logged at `debug`, and every command still renders exactly as it did. So the emission problems below are unchanged. The two styling backends still disagree — `colored` internally honors `NO_COLOR` and TTY state; `comfy_table::Color` writes escape sequences unconditionally — so `NO_COLOR=1 orbit task list` still emits color from the table path, redirecting a table command to a file still captures ANSI and box-drawing glyphs, and `comfy-table` still takes width from the terminal even when there is no terminal. **Still diverges from [./specs/output-modes.md](./specs/output-modes.md) §1** until a renderer reads the sink [ADR-0306], [ADR-0308].

## 6. Per-Command Structured Output

`--json` is declared independently on each command as `#[arg(long)] pub json: bool` and handled by an `if self.json { … } else { … }` branch inside `execute()`. 86 of 150 argument structs carry it; 64 have no machine-readable path.

Because both branches are written by hand in the same function, they drift. `orbit tool list` emits seven JSON fields (`name`, `description`, `enabled`, `active`, `status`, `builtin`, `parameters`) and five table columns, and the table's `REQUIRED INPUT` column is a summary string produced by `format_required_tool_input_summary` that exists in no payload field. There is also no `ndjson` mode: `--json` on a list command emits one pretty-printed array, which a streaming consumer must buffer whole. **Diverges from [./specs/output-modes.md](./specs/output-modes.md)** [ADR-0306].

A global `--format auto|table|json|ndjson` is accepted on every command that does not already own a `--format` — `audit export` and `hook pretooluse` do, with unrelated value types, and keep theirs. It resolves a mode through §2's precedence and no further: no command body reads the result yet, so passing it is currently inert. `--json` remains the only flag that changes output, unchanged byte for byte [ORB-10569].

## 7. Empty States and Errors

Empty results are handled inconsistently. `command/policy/list.rs` and `command/task/artifacts.rs` print a sentence (`"No policy definitions found."`); most list commands print a header row with no body beneath it. Errors are routed through `main.rs::print_error`, which emits the JSON error payload when the command declared a JSON preference and a plain message otherwise — this is the one place in the CLI where output mode is resolved centrally rather than inline, and it is the shape the rest of the surface should follow.

Errors are printed to stdout on the JSON path via `json::print_with_format`, not to stderr.

## 8. Test Coverage of Output

There are no output snapshots. `crates/orbit-cli/src/snapshots/` holds a single file, `audit_guard_event_json_shapes.json`, covering audit event JSON shape; `crates/orbit-cli/tests/snapshots/` holds one more, `mcp_tools_list.json`, covering the MCP tool listing. Both assert on JSON.

The first rendering assertions arrived with the borderless migration [ORB-10567] and are written as behavior, not golden files. `src/output/tests/table.rs` renders at pinned widths — passed as an argument rather than read from `COLUMNS`, so a `--nocapture` run cannot change the geometry — and asserts line count, gutter, truncation, alignment, column suppression, and column dropping. `tests/table_rendering.rs` runs the binary and asserts that an *N*-record `orbit tool list --all` and `orbit task list` are *N* body lines under one header with no box glyphs, and that a zero-result list leaves stdout empty. Nothing yet asserts on ANSI emission.

## 9. Concerns & Honest Limitations

**The specs describe a target the code does not implement.** Every §-level divergence above is real and unfixed as of this doc's `last_validated` date. Reading only `specs/` will misrepresent current behavior.

**The refactor's cost is concentrated in a trait signature.** `Execute::execute` returns `Result<(), OrbitError>` and writes to stdout as a side effect. Making commands return payloads changes that signature across all 154 `impl Execute` blocks, which is a single mechanical change but an unavoidably wide one. It cannot be landed incrementally per command without a transitional dual path.

**Terminal width detection needed no new dependency.** `comfy-table` still resolves width internally for the tables it draws; the sink asks `TIOCGWINSZ` through the `libc` dependency `orbit-cli` already carries on unix, after `COLUMNS`. Non-unix has no query and falls back to `COLUMNS` alone. The 0 fallback ("do not truncate") is asserted in the sink's tests but still untested against a real consumer, because nothing renders through the sink yet.

**The role vocabulary is lossy on purpose and may be wrong.** Collapsing `in-progress`, `review`, and `proposed` into `active`/`neutral` discards distinctions operators may be reading today. No one has checked whether the current colors are load-bearing in practice.

**Accessibility is unaudited.** The palette has never had a contrast check against common terminal themes, and the specs' "color is never the sole carrier" rule is asserted, not verified.

**Nothing is enforced.** These are documents. There is no lint that rejects a new `comfy_table::Table` in a command body or a new bare `println!` of tabular data.

## Task References

- [T20260411-0335] — introduced the dynamic full-width table arrangement that produces the current wrapping.
- [T20260427-43] — added a `friction` arm to both halves of the duplicated color vocabulary.
- [ORB-00279] — flattened the `orbit-cli` command tree into the current per-command module layout.
- [ORB-10202] — removed the `friction` arm from both halves when the status was retired.
- [ORB-10228] — added trusted MCP session context to the audit event JSON payload, but not to the printed line.
- [ORB-10356] — made `OrbitError` `#[non_exhaustive]`, adding the `internal_error` catch-all to the payload's `code` discriminator.
- [ORB-10569] — introduced `output/sink.rs` and the global `--format`, resolved once per invocation and not yet consumed.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

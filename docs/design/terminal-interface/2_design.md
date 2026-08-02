---
title: Terminal Interface — Design
owner: claude
last_updated: 2026-08-01
last_validated: 2026-08-01
status: Draft
feature: terminal-interface
doc_role: design
type: design
summary: "Current orbit-cli output implementation — comfy-table bordered tables, duplicated color vocabularies, per-command --json — and where it diverges from the specs."
tags: [terminal-interface]
paths: ["crates/orbit-cli/src/output/**", "crates/orbit-cli/src/command/**"]
related_features: [terminal-interface]
related_artifacts: [ADR-0306, ADR-0307, ADR-0308]
---

# Terminal Interface — Design

This document describes what `orbit-cli` renders today. It is deliberately a record of the current implementation, not of the target: the target is prescribed by [./specs/](./specs/) and decided in [./4_decisions.md](./4_decisions.md). Where current behavior diverges from a spec, this doc says so and names the spec, so a reviewer can tell an intentional gap from a regression. Web dashboard rendering is out of scope and lives in [user-interface/2_design.md](../user-interface/2_design.md).

## 1. The Output Module

`crates/orbit-cli/src/output/` has three submodules and 217 lines total. `table.rs` builds `comfy_table::Table` values, `color.rs` maps domain values to styles, `json.rs` serializes payloads and error envelopes. There is no renderer abstraction above them: they are helper functions that command bodies call directly.

`json.rs` is the closest thing to a contract. It owns `error_payload`, which projects an `OrbitError` into `{error, code}` plus optional `did_you_mean`, `artifact_origin`, and task-bundle corruption fields. Error output is therefore already payload-shaped in a way that normal output is not. Two caveats: the `code` discriminator gained a `_ => "internal_error"` catch-all when `OrbitError` became `#[non_exhaustive]` [ORB-10356], so an unmapped variant degrades silently rather than failing to compile; and a `RemoteTool` error returns the remote payload verbatim, bypassing the shape entirely, so a consumer cannot assume `code` is present.

## 2. Table Construction

`build_table` is the single constructor, used by 21 command modules. It applies four settings: the `UTF8_BORDERS_ONLY` preset, `ContentArrangement::DynamicFullWidth`, `…` as the truncation indicator, and a bold header row.

The preset draws the outer box and the header rule visible in `orbit tool list`. The arrangement expands the table to the terminal width and, on overflow, **wraps** rather than truncates — it was introduced to make tables survive narrow terminals [T20260411-0335], and it does, at the cost described below. The truncation indicator only takes effect on rows whose height is capped, which is what `add_single_line_row` does via `Row::max_height(1)` — and only 2 of the 21 call sites use it (`command/routine/list.rs`, `command/task/output.rs`). The other 19 add rows directly and wrap.

The practical result is that a row is not a line. `orbit tool list` wraps `REQUIRED INPUT` and `DESCRIPTION` across up to four lines each, which is what makes the border look load-bearing: with variable-height rows, a rule is the only row separator. **Diverges from [./specs/table-rendering.md](./specs/table-rendering.md)**, which requires a borderless preset and a one-line row invariant [ADR-0307].

## 3. Hand-Padded Line Output

Not every list goes through `build_table`. `orbit audit list` prints each event through `print_audit_event_line` in `command/audit/support.rs`, a single `println!` with the format string `"[{}] {:<8} {:<6} {}:{:<20} {}ms"`.

The column widths are literals, not functions of the result set, so alignment holds only while every value fits — a tool name longer than 20 characters shifts the duration column for that row alone. There is no header, so the columns are unlabeled. The duration is left-aligned with a trailing unit, so `187ms` and `0ms` do not align on their magnitudes. **Diverges from [./specs/table-rendering.md](./specs/table-rendering.md) §2 and §3** (computed widths, right-aligned numerics).

98 modules under `command/` call `println!` directly for some part of their output, so this is a pattern rather than an isolated case — a consequence of the flattened per-command layout [ORB-00279], where each module owns its rendering end to end.

This file is also where the drift between the two renderings is easiest to see. [ORB-10228] added trusted session-context fields (`workspace_id`, `caller_machine_id`, `transport`, `mcp_call_id`) to `audit_event_to_json` in the same file, and left `print_audit_event_line` untouched. The JSON view gained provenance the human view has never shown, and nothing flagged the asymmetry.

## 4. Two Color Vocabularies

`color.rs` defines the same mappings twice in incompatible types. Five functions — `status_color_cell`, `priority_color_cell`, `job_state_color_cell`, `doctor_status_color_cell`, `task_type_color_cell` — return `comfy_table::Cell` values for table rows. Three — `status_color`, `priority_color`, `job_state_color` — return `String` values styled by the `colored` crate for line output. The backend is chosen by the call site's rendering shape, not by the value's meaning. `doctor_status_color_cell` and `task_type_color_cell` have no String counterpart, so the pairing is not even consistent; `task_type_color_cell` applies no styling at all and exists only to satisfy the cell-returning shape.

The copies have already drifted: `status_color` maps `backlog` explicitly, `status_color_cell` does not. The two-edit cost is visible in the history — [T20260427-43] added a `friction` arm to both functions, and [ORB-10202] removed it from both when the status was retired. The palette itself is broader than a closed role set — `in-progress` is cyan, `review` is magenta, `done` is bold green — so it encodes more distinctions than the target vocabulary. **Diverges from [./specs/color-and-styling.md](./specs/color-and-styling.md)** [ADR-0308].

## 5. No Sink Resolution

The CLI never asks whether stdout is a terminal, with one exception: `command/log/tail.rs` calls `stdout.is_terminal()` to decide whether to colorize the tailed stream. That check is local to the command and not reused.

Consequently the two styling backends disagree about emission. `colored` internally honors `NO_COLOR` and TTY state; `comfy_table::Color` writes escape sequences unconditionally. `NO_COLOR=1 orbit task list` still emits color from the table path. Redirecting any table command to a file captures ANSI and box-drawing glyphs. Width is taken from the terminal even when there is no terminal. **Diverges from [./specs/output-modes.md](./specs/output-modes.md) §1** [ADR-0306], [ADR-0308].

## 6. Per-Command Structured Output

`--json` is declared independently on each command as `#[arg(long)] pub json: bool` and handled by an `if self.json { … } else { … }` branch inside `execute()`. 86 of 150 argument structs carry it; 64 have no machine-readable path.

Because both branches are written by hand in the same function, they drift. `orbit tool list` emits seven JSON fields (`name`, `description`, `enabled`, `active`, `status`, `builtin`, `parameters`) and five table columns, and the table's `REQUIRED INPUT` column is a summary string produced by `format_required_tool_input_summary` that exists in no payload field. There is also no `ndjson` mode: `--json` on a list command emits one pretty-printed array, which a streaming consumer must buffer whole. **Diverges from [./specs/output-modes.md](./specs/output-modes.md)** [ADR-0306].

## 7. Empty States and Errors

Empty results are handled inconsistently. `command/policy/list.rs` and `command/task/artifacts.rs` print a sentence (`"No policy definitions found."`); most list commands print a header row with no body beneath it. Errors are routed through `main.rs::print_error`, which emits the JSON error payload when the command declared a JSON preference and a plain message otherwise — this is the one place in the CLI where output mode is resolved centrally rather than inline, and it is the shape the rest of the surface should follow.

Errors are printed to stdout on the JSON path via `json::print_with_format`, not to stderr.

## 8. Test Coverage of Output

There is effectively none. `crates/orbit-cli/src/snapshots/` holds a single file, `audit_guard_event_json_shapes.json`, covering audit event JSON shape; `crates/orbit-cli/tests/snapshots/` holds one more, `mcp_tools_list.json`, covering the MCP tool listing. Both assert on JSON. No test asserts on rendered table text, column alignment, truncation, or ANSI emission.

This cuts both ways: the migration prescribed by the specs will not be blocked by snapshot churn, but it also has nothing to verify against, so the acceptance work has to build its own fixtures.

## 9. Concerns & Honest Limitations

**The specs describe a target the code does not implement.** Every §-level divergence above is real and unfixed as of this doc's `last_validated` date. Reading only `specs/` will misrepresent current behavior.

**The refactor's cost is concentrated in a trait signature.** `Execute::execute` returns `Result<(), OrbitError>` and writes to stdout as a side effect. Making commands return payloads changes that signature across all 154 `impl Execute` blocks, which is a single mechanical change but an unavoidably wide one. It cannot be landed incrementally per command without a transitional dual path.

**Terminal width detection is not currently a dependency.** `comfy-table` resolves width internally. Moving width into an explicit sink means taking a direct dependency on `terminal_size` or equivalent, and deciding the fallback width when there is no terminal (the specs say 0 — emit untruncated — but that is a claim not yet tested against real consumers).

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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

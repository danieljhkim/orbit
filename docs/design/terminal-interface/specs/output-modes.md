---
type: design
summary: "Spec: Output Modes and Sink Resolution"
last_validated: 2026-08-01
---

# Spec: Output Modes and Sink Resolution

Every `orbit` command produces a structured payload and hands it to a renderer. The renderer resolves one output mode — `auto`, `table`, `json`, or `ndjson` — from global flags, environment, and the properties of the sink, once per invocation. A command body never decides how it is displayed, never asks whether stdout is a terminal, and never writes to stdout directly.

## Why This Exists

Structured output is currently a per-command opt-in on 86 of 150 argument structs, hand-written alongside the human rendering in the same function, and the two drift. Nothing detects whether stdout is a terminal, so a pipe receives box-drawing characters and ANSI escapes. Both follow from rendering decisions living in command bodies. Rationale in [ADR-0306].

## 1. The Sink

The sink is resolved once at startup and answers every environment question. Nothing downstream may re-derive these.

| Property | Source | Fallback |
|----------|--------|----------|
| `is_tty` | `IsTerminal` on stdout | `false` |
| `width` | `COLUMNS` env, else terminal query | `0` (means: do not truncate) |
| `color_allowed` | see [./color-and-styling.md](./color-and-styling.md) §2 | `false` |

**Invariant:** `is_tty == false` implies `width == 0` and `color_allowed == false`. A non-terminal sink is never width-adapted or styled, regardless of what `COLUMNS` says.

`width == 0` disables truncation rather than assuming 80. Assuming a width for a sink that has none produces silently truncated data in a file, which is worse than a long line.

## 2. Mode Resolution

Precedence, first match wins:

1. `--format <mode>` explicitly passed.
2. `--json` (per-command legacy alias) → `json`.
3. `ORBIT_FORMAT` environment variable.
4. `auto`.

`auto` resolves to `table` when `is_tty`, and to the **plain** form otherwise. Plain is `table` with the header suppressed, borders and ANSI absent, truncation disabled, and single-tab field separators — the form `cut -f` expects. Plain is a rendering of `table`, not a fourth mode a command can request.

`--format` is a global argument declared once on the root command, not redeclared per subcommand. The existing per-command `--json` booleans remain accepted and hidden from help; they are not removed [ADR-0306].

## 3. Mode Contracts

| Mode | Shape | Truncates | Color | Streams |
|------|-------|-----------|-------|---------|
| `table` | aligned columns, header | yes, to `width` | if allowed | no |
| plain | tab-separated, no header | no | no | no |
| `json` | one document | no | no | no |
| `ndjson` | one document per line | no | no | **yes** |

- `json` for a list command emits a single array; for a detail command, a single object. It is pretty-printed only when `is_tty`.
- `ndjson` emits one complete JSON value per line and flushes per record. It is the only mode that may produce output before the command has finished collecting results, and the only correct choice for a long or unbounded list.
- `table` and `json` for the same invocation describe the same records. The table may omit fields and reformat values; it may not contain a value absent from the payload, and it may not omit a record the payload includes.

## 4. Payload Rules

- A payload is serializable and self-describing. Field names are `snake_case` and stable — renaming one is a breaking change to the CLI's contract, on the same footing as renaming a flag.
- Absent values are `null`, not omitted, so a consumer indexing by key does not have to distinguish "missing" from "not applicable".
- Timestamps are RFC 3339 with an explicit offset. Durations are integers in a unit named by the field (`duration_ms`), never pre-formatted strings.
- Values are typed as they mean: a count is a number, not a string. Formatting a number for display is the renderer's job.
- Enum-like values (status, state, role) are the canonical lowercase token. Display casing is applied at render time.

## 5. Streams

- **stdout carries the payload and nothing else.** Progress, warnings, counts, empty-state prose, and diagnostics go to stderr, in every mode.
- **Errors go to stderr.** In `json`/`ndjson` modes an error is a single JSON object on stderr using the existing `error_payload` shape (`error`, `code`, and the optional `did_you_mean` / `artifact_origin` / task-bundle fields); in other modes it is a plain message. This is a change from current behavior, which writes the JSON error payload to stdout.
- **Exit codes are load-bearing.** `0` success, `1` command failure, `2` usage error. A command that printed an error object must not exit `0`.
- **A broken pipe is not an error.** `EPIPE` on stdout exits `0` silently — `orbit task list | head` must not print a panic.

## 6. Progress

- Spinners, progress bars, and status tickers are emitted only when `is_tty` and only to stderr.
- Progress output is erased before the process exits; a redirected stderr must not accumulate carriage-return frames.
- No progress in `json` or `ndjson`, even on a TTY. Those modes are chosen by consumers who are not watching.

## 7. Migration

1. Introduce the sink and the global `--format`; leave every command body untouched. `auto` initially resolves to today's rendering, so nothing changes yet.
2. Route the existing per-command `--json` branches through the resolver so precedence is centralized. Still no behavior change for explicit callers.
3. Convert command bodies to return payloads. This changes `Execute::execute`'s signature across all 154 `impl Execute` blocks and is the expensive step; it can be staged with a transitional trait method that defaults to the current side-effecting path.
4. Once a command returns a payload, delete its inline table construction and let the renderer own it.
5. Move error output to stderr and audit exit codes.

Step 5 is the only user-visible break for existing scripts (an error object moves from stdout to stderr). Call it out in the changelog rather than shipping it quietly.

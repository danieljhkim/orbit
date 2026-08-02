## Context

Every `orbit` subcommand renders its own human output inline. The pattern repeats across `crates/orbit-cli/src/command/`: build a `serde_json::Value` when `--json` is passed, otherwise build a `comfy_table::Table` or `println!` a hand-padded line, in the same `execute()` body. Two consequences follow.

First, the two renderings drift. `orbit tool list` emits seven JSON fields but five table columns. `orbit audit list` emits a fixed-width line (`"[{}] {:<8} {:<6} {}:{:<20} {}ms"` in `command/audit/support.rs`) whose padding is a literal in a format string rather than a function of the data, so a tool name longer than 20 characters silently breaks the column the operator is scanning. The drift also accumulates unnoticed: [ORB-10228] added trusted session-context fields to `audit_event_to_json` in that same file and left the printed line untouched, so the JSON view carries provenance the human view has never shown.

Second, structured output is opt-in per command. 86 of 150 `#[derive(Args)]` structs carry a `pub json: bool` field, each declared and handled independently. The remaining 64 have no machine-readable path at all, and the flag's presence is not discoverable without reading the help for each command.

Nothing in the CLI detects whether stdout is a terminal, except `command/log/tail.rs`, which does it locally for one colorized stream. So `orbit tool list | grep` receives the same box-drawn, width-adapted, ANSI-styled output a human sees.

The one place output mode is already resolved centrally is `main.rs::print_error`, which routes an `OrbitError` through `output/json.rs::error_payload` when the command declared a JSON preference. That function is the closest thing the CLI has to an output contract, and this decision generalizes its shape to normal output.

## Decision

A command produces a structured payload; rendering is a separate layer that consumes it. Command bodies stop constructing tables and format strings.

- The payload is the contract. The human rendering is a projection of it and may only drop or reformat fields, never introduce a value the payload does not carry.
- Output mode is resolved centrally, not per command: an explicit global `--format` (`auto|table|json|ndjson`) wins; otherwise `auto` renders the table form when stdout is a TTY and the plain machine form when it is not.
- The existing per-command `--json` flags stay as accepted aliases for `--format json`. They are not removed.
- Piped output is plain: no borders, no ANSI, no width adaptation to a terminal that isn't there.

Rejected alternative: add `--json` to the 64 commands that lack it and leave the rendering inline. Rejected because it treats the symptom. The drift between the JSON and table views is caused by their being built independently in the same function; adding more independent pairs makes the invariant harder to hold, and still leaves a piped `orbit tool list` emitting box-drawing characters.

Rejected alternative: make `--format json` the default and have humans opt into tables. Rejected because it degrades the far more common interactive case to serve scripts, which can set the flag explicitly and, under this decision, get the right thing from a pipe anyway.

## Consequences

- A piped or redirected `orbit` command is parseable by `cut`/`awk` without flags, which is the property that currently fails.
- New commands get structured output for free, so the 64-command gap closes as a consequence of the refactor rather than as 64 separate changes.
- The JSON and human views cannot disagree about a field's value, because one is derived from the other.
- Cost: every command that prints must be refactored to return a payload instead of writing to stdout, and `Execute::execute` returning `Result<(), OrbitError>` no longer expresses that — the trait signature changes, touching all 154 `impl Execute` blocks.
- Cost: output becomes environment-dependent. A command run in CI (no TTY) prints differently than the same command run in a terminal, so any test or runbook that asserts on stdout must pin `--format` explicitly or it will pass locally and fail in CI.
- Cost: moving the JSON error payload from stdout to stderr is a breaking change for any existing script that parses errors off stdout, and there is no output snapshot suite that would catch the regression — `crates/orbit-cli/src/snapshots/` holds one file, covering audit event JSON shape only.
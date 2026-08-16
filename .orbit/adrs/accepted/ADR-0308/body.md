## Context

`crates/orbit-cli/src/output/color.rs` defines the same status vocabulary twice, in two incompatible type systems. `status_color_cell` returns a `comfy_table::Cell` for table rows; `status_color` returns a `String` styled by the `colored` crate for line output. `job_state_color_cell`/`job_state_color` and `priority_color_cell`/`priority_color` are the same duplication. Two further cell-returning functions, `doctor_status_color_cell` and `task_type_color_cell`, have no String counterpart at all — and `task_type_color_cell` applies no styling whatsoever, existing only to satisfy the cell-returning shape. Eight functions, three genuine pairs, no consistent rule about which values get a pair.

The duplication exists because the styling backend is chosen by the call site's rendering shape rather than by the meaning of the value. It means every new status needs two edits, and nothing detects when only one lands. The two-edit cost shows up in the history: [T20260427-43] added a `friction` arm to both halves, and [ORB-10202] removed it from both when the status was retired. The copies have nevertheless drifted — the string form maps `backlog` explicitly, the cell form does not.

The two backends also disagree about when to emit ANSI at all. The `colored` crate honors `NO_COLOR` and TTY detection internally. `comfy_table::Color` does not — it writes escape sequences unconditionally. So `NO_COLOR=1 orbit task list` still emits color from the table path, and a redirect captures escape sequences into the file. Nothing in the CLI sets a global override; the only TTY check in the crate is a local one in `command/log/tail.rs`.

## Decision

Color is a semantic token attached to a value's meaning, resolved once at the sink.

- A single vocabulary maps domain values to a small closed set of roles: `ok`, `warn`, `error`, `active`, `muted`, `neutral`. Commands tag a value with a role; they never name a color or call a styling crate.
- The renderer resolves role to ANSI, and is the only place either styling backend is touched. Adding a status is one edit.
- Emission is decided once, at the sink, in this precedence: `--no-color` or `NO_COLOR` (any non-empty value) disables; `--color=always` forces; otherwise color is on only when stdout is a TTY.
- Color is never the sole carrier of meaning. A status cell prints its word, and the word is legible with color stripped — this is what makes the `NO_COLOR` and piped paths correct rather than merely degraded.
- Roles apply to values, not rows. A failed row is not painted red; its status cell is.

Rejected alternative: keep the eight functions and add a test asserting the pairs agree. Rejected because it preserves the two-edit requirement, only catches drift after it is written, and says nothing about the two functions that have no pair to compare against.

Rejected alternative: drop `colored` and route all line output through `comfy-table`. Rejected because `comfy_table::Color` is the backend that ignores `NO_COLOR`, and single-value output has no table to build.

## Consequences

- `NO_COLOR=1` and redirection are honored everywhere, not only on the paths that happen to use `colored`.
- A new status or run state is defined once, and cannot appear styled in one view and unstyled in another.
- The closed role set bounds the palette, which is the precondition for a contrast audit; an open set of per-command colors could not be audited.
- Cost: a status whose role is not obvious now forces a judgment at definition time (`review` is neither `ok` nor `warn`), and mapping it to `neutral` loses a distinction the current code encodes as magenta. The vocabulary is deliberately smaller than what exists.
- Cost: the role indirection means reading the source no longer tells you what color a value prints; that requires reading the resolver too.
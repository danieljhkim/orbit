# orbit-cli

Project instructions for the clap-based CLI entry point.

## Command tree convention

`crates/orbit-cli/src/command/` follows one rule:

**Directory ⟺ one command.** A subdirectory under `command/` IS a command
(`orbit <name>`) — never a grouping of unrelated commands. Its clap struct and,
when it has subcommands, its `Subcommand` enum live in `command.rs`. `mod.rs` is
module declarations and re-exports only — no clap derives, no command bodies.
Each subcommand body lives in its own sibling file (`<subcommand>.rs`).

A command with no subcommands may still own a directory when it needs modules
of its own — an adapter between the terminal/host and a domain crate, say. The
test is the same one: everything under the directory serves that one command.

**Single `.rs`** is fine for any command that fits comfortably in one file,
whether it has subcommands inline or none at all. No minimum-files threshold
forces a directory.

### Reference shapes

- [`skill/`](src/command/skill) — five subcommands, one `.rs` per body, no
  shared helper file needed.
- [`audit/`](src/command/audit) — five subcommands, one `.rs` per body, shared
  helpers in `support.rs`.
- [`task/`](src/command/task) — large surface with `artifact` nested parent
  and a `tests/` subdir mirroring source files.
- [`init/`](src/command/init) — no subcommands, but owns the host-facing init
  adapter: `agent_detect.rs` probes `PATH` for provider CLIs, `agent_prompt.rs`
  reads crew choices from stdin, and `seed.rs` turns both into the explicit
  `orbit_config::ConfigSeed` handed to Core [ORB-10885].

### What `mod.rs` may contain

- `mod xxx;` / `pub mod xxx;` declarations.
- `pub use command::{XxxCommand, XxxSubcommand};` (and other internal
  re-exports the crate needs).
- `#[cfg(test)] mod tests;`.

That's it. If you find yourself reaching for `#[derive(Subcommand)]` inside
`mod.rs`, move it to `command.rs` instead.

### What `command.rs` contains

- The parent `XxxCommand` `#[derive(Args)]` struct with `#[command(subcommand)]`.
- `impl Execute for XxxCommand` (delegates to the enum).
- The `XxxSubcommand` `#[derive(Subcommand)]` enum.
- `impl Execute for XxxSubcommand` (dispatches to each subcommand's
  `Args::execute`).
- Anything tightly coupled to the parent surface itself (e.g. a custom
  `help_template`, `RUN_AFTER_HELP` strings).

Helper functions shared across multiple sibling subcommand files belong in a
neutral file (`support.rs` is the convention) rather than `command.rs`, so
the parent file stays focused on dispatch.

## --help grouping is a render concern, not a filesystem concern

The grouped sections you see in `orbit --help` (Environment / Operate /
Observe / Definitions / Services) come from a hand-rolled `help_template` in
[`command/mod.rs`](src/command/mod.rs) — not from the source tree. Clap's
derive macros do not support per-variant `help_heading` on enum variants
(`subcommand_help_heading` only renames the single `Commands:` block), so
the template renders the grouping manually.

When you add a new top-level command:

1. Add it to the `Commands` enum in `command/mod.rs` in the variant order
   that matches its template section. The variant order also determines
   where a missing-from-template command would appear by default.
2. Add the row to the matching section in the `help_template` string.
3. Add exactly one exhaustive operation arm in
   [`command/operation.rs`](src/command/operation.rs). That arm declares the
   command's dispatch, runtime need, audit metadata, JSON error preference,
   and hook error policy together; do not add a default arm or recreate any
   of those policy matches in `main.rs` or `audit_middleware.rs`.

The source tree stays flat — never create a grouping subdirectory under
`command/` to mirror the visual grouping. Past attempts (`definitions/`,
`environment/`, `observe/`) made it impossible to tell from `ls` whether a
directory was a parent command or a folder, and were removed in ORB-00279.

## Registry-derived commands

Some parent commands are **not** hand-written clap structs. `friction` is
derived from an operation registry declared once in `orbit-common`
(ADR-0209 bearing 1, ORB-10358): `command/operation_args.rs` builds the
subcommand tree, the args, and the tool input from that registry, and
`command/friction.rs` holds only the trait glue and the response renderers.
Adding a friction verb is a registry entry plus a handler — no edit under
`command/`, including no new arm in `command/operation.rs`, whose friction arm
reads the invocation instead of matching verb by verb.

`command/operation_args.rs` is generic over the noun's verb type, so the next
noun to migrate reuses it unchanged. Before migrating one, read
[docs/design/operations-as-data/references/cookbook.md](../../docs/design/operations-as-data/references/cookbook.md)
— in particular Step 0, which freezes the current `--help` output as fixtures
before any code moves. That is the only thing that makes "argv is unchanged"
checkable rather than hopeful.

Two idioms therefore coexist under `command/`, and that is expected (ADR-0209's
adoption model). Hand-written clap is still correct for a command that has not
been migrated; do not half-migrate one.

## Crate boundary

`orbit-cli` is a clap entry point. Domain logic lives in `orbit-core` and
focused feature crates such as `orbit-mcp`, `orbit-registry`, and `orbit-web`. CLI
subcommand files hold only:

- Clap `Args` / `Subcommand` definitions.
- One `impl Execute` that calls into the owning domain crate.
- Optional `println!` / `eprintln!` for stdout/stderr formatting.
- Output projection helpers (JSON shaping, table rendering) — these are
  presentation concerns, not domain logic.

Anything beyond that — registry lookups, file I/O, audit decisions, state
mutation — belongs in the owning domain crate. See [`ARCHITECTURE.md`](../../ARCHITECTURE.md)
for the full crate-layer rules.

# orbit-cli

Project instructions for the clap-based CLI entry point.

## Command tree convention

`crates/orbit-cli/src/command/` follows one rule:

**Directory ⟺ parent command.** A subdirectory under `command/` IS a parent
command (`orbit <name>`) that owns subcommands. Its parent struct and
`Subcommand` enum live in `command.rs`. `mod.rs` is module declarations and
re-exports only — no clap derives, no command bodies. Each subcommand body
lives in its own sibling file (`<subcommand>.rs`).

**Single `.rs`** is fine for any command that fits comfortably in one file,
whether it has subcommands inline or none at all. No minimum-files threshold
forces a directory.

### Reference shapes

- [`hook/`](src/command/hook) — three subcommands, one `.rs` per body, shared
  enum types in `render.rs`.
- [`learning/`](src/command/learning) — ten subcommands, a `comment` parent
  with its own nested subcommands, shared formatting in `output.rs`.
- [`task/`](src/command/task) — large surface with `artifact` nested parent
  and a `tests/` subdir mirroring source files.

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
3. Add the dispatch arm to `impl Execute for Commands`.

The source tree stays flat — never create a grouping subdirectory under
`command/` to mirror the visual grouping. Past attempts (`definitions/`,
`environment/`, `observe/`) made it impossible to tell from `ls` whether a
directory was a parent command or a folder, and were removed in ORB-00279.

## Crate boundary

`orbit-cli` is a clap entry point. Domain logic lives in `orbit-core`. CLI
subcommand files hold only:

- Clap `Args` / `Subcommand` definitions.
- One `impl Execute` that calls into `orbit-core`.
- Optional `println!` / `eprintln!` for stdout/stderr formatting.
- Output projection helpers (JSON shaping, table rendering) — these are
  presentation concerns, not domain logic.

Anything beyond that — registry lookups, file I/O, audit decisions, state
mutation — belongs in `orbit-core`. See [`ARCHITECTURE.md`](../../ARCHITECTURE.md)
for the full crate-layer rules.

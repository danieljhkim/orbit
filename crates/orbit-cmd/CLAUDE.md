# orbit-cmd

Project instructions for the shared application-composition crate.

## One job

`orbit-cmd` exists to solve exactly one problem: **`orbit-registry` sits above
`orbit-core` in the crate graph, so Core cannot see the machine's workspace
catalog.** Any command group that needs both a Core runtime and Registry state
lives here, so that neither lower-layer dependency has to be reversed.

Both `orbit-cli` and `orbit-web` consume it. A command group whose logic is
needed by Core's own runtime internals (tool hosts, engine hosts, bootstrap
seeding) belongs in `orbit-core::adapter::command`, not here.

## Layout: one flat module per command group

The source tree is deliberately flat — [`doctor.rs`](src/doctor.rs),
[`migrate.rs`](src/migrate.rs), [`registry_runtime.rs`](src/registry_runtime.rs),
[`registry_routines.rs`](src/registry_routines.rs),
[`workspace_catalog.rs`](src/workspace_catalog.rs),
[`task_owner.rs`](src/task_owner.rs), [`diagnostics.rs`](src/diagnostics.rs),
[`activity_v2.rs`](src/activity_v2.rs), [`agent_rules.rs`](src/agent_rules.rs).
Each file owns one group and states, in its module doc, which boundary it
closes. Give a group a directory only when it genuinely grows sibling modules
of its own; do not create a grouping directory that mirrors the CLI's `--help`
sections.

Each module names exactly one composition seam. `workspace_catalog` turns a
`WorkspaceScope` into registered checkouts for Core's federated search;
`task_owner` is the single owner of "which registered workspace owns this task
ID"; `registry_runtime` builds Core's runtime binding from a registered
checkout. If a new file cannot be described that way in one sentence, it is
probably two files or belongs in another crate.

## Runtime methods are extension traits

Behavior that would once have been an inherent `impl OrbitRuntime` block is
exposed as a per-module `*Commands` trait (`DoctorCommands`, `MigrateCommands`,
`ActivityV2Commands`, `DiagnosticsCommands`), re-exported through
[`prelude`](src/lib.rs). Add a new group the same way, and add its trait to the
prelude in the same change.

## Pure consumer of Core's public API

Every module here is a consumer of `orbit_core::OrbitRuntime`'s **public**
surface. When something you need is not exposed:

1. Add the seam in `orbit-core` deliberately, with the visibility widening
   justified there.
2. Then consume it here.

Never reach around Core to re-implement a rule it owns. Validation,
authorization, audit decisions, and persistence invariants stay in Core and
`orbit-store`; this crate composes and projects.

Equally, nothing here may become presentation. There is no `clap` and no
`axum` dependency, and there must not be one — argv parsing and help text stay
in `orbit-cli`, HTTP handlers in `orbit-web`. Returning a plain result struct
that both can render is the point.

## Assets

[`assets/agent-rules.md`](assets/agent-rules.md) is the self-contained block
`orbit workspace init --inject-agent-rules` writes into a workspace's
`CLAUDE.md` / `AGENTS.md`. Its start/end markers live literally inside the
asset because injection is re-runnable and must find its own previous output —
edit the asset, not the marker handling, when the rule text changes.

## Tests

Unit tests live in [`src/tests/`](src/tests) mirroring source filenames
([`test_layout.md`](../../docs/design-patterns/test_layout.md)). There is no
crate-root `tests/` directory: end-to-end coverage of these command groups
belongs to the CLI's integration tests, which exercise the same code through
the real entry point.

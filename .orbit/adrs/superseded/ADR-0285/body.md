## Context

ADR-0198 cut the agent graph surface to orbit-graph (v2) and, in doing so, removed the `orbit graph` CLI command — agents reach the graph in-process over MCP, and direct CLI users were pointed at the standalone `orbit-graph-cli` binary. In practice that binary is not always on `PATH` (the agent shell documented in [`plugin/agents/orbit-code-reader.md`](../../../plugin/agents/orbit-code-reader.md) notes "`orbit-graph-cli` is not on PATH in this environment"), leaving a shell user who holds only the `orbit` binary with no command-line path to the graph. Every other Orbit capability is reachable from the single `orbit` binary; the graph was the lone exception.

## Decision

Reintroduce `orbit graph` as a thin wrapper over the `orbit-graph-cli` command layer. `orbit-graph-cli` is lib-ified (lib + bin): its `Command` subcommand enum and `Command::run` dispatch move into a library surface that both the standalone binary and `orbit-cli` consume, so there is exactly one command layer and no duplication. `orbit-cli` embeds that enum under an `orbit graph` parent and prints the same JSON the standalone binary emits, mapping the graph CLI error into `OrbitError`. The graph subcommands stay worktree-scoped (the DB is discovered from the current git worktree) and do not route through `OrbitRuntime`. This amends only ADR-0198's "there is no `orbit graph` subcommand" consequence; the v2 cutover, the MCP adapter as the agent surface, and the removal of `orbit-knowledge` are unchanged.

## Consequences


- `orbit graph {sync, search, show, refs, callees, impact, trace, overview, implementors, deps, version, db-path, clean}` is available from the single `orbit` binary; output matches the standalone `orbit-graph-cli` (same library, same compact JSON).
- New crate edge `orbit-cli → orbit-graph-cli` (recorded in [`ARCHITECTURE.md`](../../../ARCHITECTURE.md)). `orbit-graph-cli` now publishes a minimal library surface (`Command`, `Command::run`, `CliError`); the per-subcommand arg structs are made `pub` to keep the public enum's interface clean under `-D warnings`.
- The agent-facing graph surface is unchanged: agents still use the in-process MCP adapter, not `orbit graph`. The new subcommand is for humans/scripts holding the `orbit` binary.
- Cost: a second consumer of the orbit-graph-cli command layer means a subcommand change now ripples to two front ends' help/output expectations. The duplication-free lib split confines the implementation to one edit site, but the orbit-cli parse tests and any `orbit graph` doc references must track the surface.

**Note (ORB-10357, 2026-07-25).** Daniel redirected: fold `orbit-graph-extract` and `orbit-graph-cli` into `orbit-graph` (it doesn't have much use and will eventually be phased out) and remove the `orbit graph` subcommand from `orbit-cli` entirely. This reverses this ADR's decision: the `orbit-cli → orbit-graph-cli` edge and the `orbit graph` subcommand it introduced are removed, not merely amended. The former `orbit-graph-cli` command layer (`Cli`, `Command`, `CommandContext`) is folded into `orbit-graph` as a `cli` module with no external caller — parked pending the graph crate's eventual deletion, per the note in [1_overview.md](./1_overview.md). No standalone binary was introduced for the consolidated crate.

## Provenance

Migrated verbatim from the local heading `orbit-graph/ADR-0199` in `docs/design/orbit-graph/4_decisions.md` by [ORB-10458]. Original status line: Superseded by ORB-10357 · 2026-06-16 · [ORB-00396] · Amends ADR-0198
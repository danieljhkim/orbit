---
type: glossary
summary: "Glossary — Orbit MCP Bridge"
last_updated: 2026-08-15
last_validated: 2026-08-15
---

# Glossary — Orbit MCP Bridge

Vocabulary specific to the ownership-aware, placement-aware Orbit MCP feature.
Standard MCP, SSH, workspace, task, and graph terms are excluded unless narrowed.
The singular-hub vocabulary this glossary previously carried — hub link, hub mode,
hub route, spoke, run lease, and the `runner` capability — is retired with
[Singular coordination hub, workspace owner, and per-run placement](../4_decisions.md#singular-coordination-hub-workspace-owner-and-per-run-placement)/[Owner-authored knowledge with hub-global IDs and explicit replicas](../4_decisions.md#owner-authored-knowledge-with-hub-global-ids-and-explicit-replicas)/[Pull-based leases with immutable placement and explicit recovery](../4_decisions.md#pull-based-leases-with-immutable-placement-and-explicit-recovery); see
[../../host-registry/4_decisions.md](../../host-registry/4_decisions.md).
Learning-specific vocabulary is likewise retired by [ORB-10736] / [Remove the native project-learning subsystem](../../project-learnings/4_decisions.md#remove-the-native-project-learning-subsystem)
and remains below only as historical terminology, not as a current contract.

| Term | Meaning |
|------|---------|
| Caller-host provenance | Stable identity of the machine running the client-facing broker, recorded separately from the machine whose process served the call. See [2_design.md §9](../2_design.md). |
| Composite route | A canonical tool with placement-specific preflight beyond a simple owner/local-derived dispatch. `orbit.search` is the only v1 example; its current implementation requires a locally owned validated checkout and executes all requested branches there. See [2_design.md §4](../2_design.md). |
| Local-derived route | Tool execution against rebuildable state derived from the exact current checkout/worktree, such as graph or docs indexes. See [2_design.md §4](../2_design.md). |
| Local MCP broker | The `orbit mcp serve` process registered with the client; preserves local checkout, resolves ownership, and dispatches canonical tools by placement. See [2_design.md §2.1](../2_design.md). |
| MCP call ID | Broker-generated correlation ID propagated to remote/local audit and returned for uncertain transport outcomes. See [2_design.md §9](../2_design.md). |
| Owned tunnel | An SSH tunnel Orbit establishes or reuses to a loopback-bound listener on a remote machine; shared infrastructure rather than one consumer's detail, and the only cross-machine mechanism in v1. See [2_design.md §5.3](../2_design.md), [Own the SSH tunnel as remote-access infrastructure, with a provisional surface over it](../4_decisions.md#own-the-ssh-tunnel-as-remote-access-infrastructure-with-a-provisional-surface-over-it). |
| Owner link | An SSH-carried MCP connection from a local broker to an owner machine's stable `machine_id`, used in v1 only for the advertised `orbit.task.*` family. See [2_design.md §5](../2_design.md). |
| Owner machine | Per workspace, the single machine declared in the machine-local `workspaces.json` as holding the canonical checkout and coordinating that workspace's records. See [host-registry/2_design.md §3](../../host-registry/2_design.md). |
| Owner-machine endpoint | The non-recursive MCP server mode that executes coordination tools for the workspaces its machine owns and refuses every other workspace with the owner named. See [2_design.md §2.2](../2_design.md). |
| Owner route | Tool execution on the machine that owns the workspace: in-process when that is this machine, otherwise refused unless the call is in the advertised task family that may cross the owner link. See [2_design.md §4.2](../2_design.md). |
| Placement class | Canonical tool metadata declaring `owner`, `local-derived`, or `composite`; independent of capability. See [2_design.md §4.1](../2_design.md). |
| Remote feature crate | `orbit-remote`, the vertical owner of registry persistence and MCP schema composition, broker, and owner route; neutral Store, MCP, Core, Tools, and Common crates do not import it. See [2_design.md §1.1](../2_design.md). |
| Task prefix | The machine-scoped namespace (`ORB`, `DE`, …) for every task ID a machine mints, chosen once at global init and immutable after; what makes IDs globally unique without an allocator. See [host-registry/2_design.md §1](../../host-registry/2_design.md). |
| Transport trust | Machine-local permission in `~/.orbit/mcp.toml` to reach a named owner machine's stable `machine_id`, pinned out of band, through a configured SSH alias. Zero or more entries; it grants a route and never ownership. See [2_design.md §1, §5.1](../2_design.md). |
| Workspace claim | An exclusive, TTL-bounded hold one operator takes to gate workflow dispatch. Distinct from ownership: ownership binds a workspace to a machine, the claim binds dispatch authority to a session. See [host-registry/2_design.md §3.2](../../host-registry/2_design.md). |

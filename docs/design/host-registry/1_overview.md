---
title: Host Registry — Overview
owner: claude
last_updated: 2026-07-18
status: Accepted
feature: host-registry
doc_role: overview
type: design
summary: First-class, validated machine identity plus a main-host inventory, enabling pull-based orchestrator-selected execution placement and a strict per-record data-placement split.
tags: [host-registry, multi-host, dispatch, routines]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**", "crates/orbit-common/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access]
related_artifacts: [ORB-00424, ORB-10248, ORB-10249, ORB-10268, ORB-10302, ORB-10319, ADR-0200, ADR-0205, ADR-0208, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240]
---

# Host Registry — Overview

The host registry makes machine identity a first-class Orbit concept: every machine
carries a validated identity (`host.toml`), the main host keeps an enumerable inventory
of all registered machines, and every record type has an explicit placement rule —
coordination records land only on the main host, knowledge records are authored by
each workspace's owner (converging outward via git), and derived indexes stay local.
The topology is a star: the hub queues, satellites poll, and no machine ever talks to
another machine. On top of that identity, the orchestrator selects *where* a task
executes — defaulting to the workspace's owner — and git-committed routine
definitions become authoritative host assignments.

The implementation is one vertical feature crate, `orbit-remote`. It owns host and
workspace identity, registry persistence, profiles and caches, MCP contract
composition, the placement broker, hub authority/link, and registration. It builds
on neutral `orbit-store` and `orbit-mcp` kernels rather than spreading one remote
feature across those crates; `orbit-cli` is the thin command/configuration edge and
`orbit-core` retains transport-independent coordination execution ([ORB-10319],
[ADR-0240]).

## 1. Motivation

Orbit today is single-machine by construction: `~/.orbit/orbit.db`, the task ID
allocator (single `local` authority), and `workspaces.json` all implicitly describe
"this box." Host identity exists only as `host_id` in `~/.orbit/host.toml` — a free
string with a hostname fallback, written as a side effect of `orbit routine init` and
consumed only by the routine sweep's host filter ([ADR-0205] kept it as "the one
genuinely host-local datum").

Three pressures force new work:

1. **Cross-machine control is landing.** The local/remote MCP unification
   ([mcp-bridge](../mcp-bridge/1_overview.md), [ORB-00424]) gives off-box clients
   the same tool surface through a placement-aware local broker and SSH-carried hub
   calls. Once satellites can write against a remote Orbit, "which
   machine owns this workspace?" and "which host should run this agent?" need
   first-class, validated answers — not an SSH alias hard-coded in
   `~/.orbit/mcp.toml`.
2. **Dispatch needs placement.** The orchestrator right-sizes `crew` per task at
   triage; it has no equivalent knob for *machine*. Execution placement should be
   selectable and validated the same way crew is.
3. **Git-committed routines need an ownership rule.** Routine definitions live in
   `.orbit/routines/` and converge through git. A pin that is a free string silently
   never fires on a typo, and an unpinned definition checked out on N source
   workspaces would fire N times. Validated host identity turns the committed pin
   into a reviewable, lintable assignment.

The registry deliberately does **not** introduce store synchronization or a second
authority — that direction was already rejected ([ADR-0200], the archived
`_archive/task-sync/` design). Placement without replication.

## 2. Core Concepts

- **Host identity** — the machine-local declaration in `~/.orbit/host.toml`: a stable
  generated `machine_id` plus a renameable human `host_id`. Names appear only in
  human-authored text and resolve through the registry at binding time; everything
  the system persists stores `machine_id`. Initialized by `orbit init`.
- **Host registry** — the inventory of known hosts kept on the main host: name,
  machine id, labels, workspace presence map (workspace → root on that host),
  last-seen, status. Enumerable via `orbit.host.list`.
- **Main host (hub)** — the machine holding the **coordination plane**: tasks,
  frictions, the run queue, the registry, and all global ID allocation, for every
  workspace regardless of who owns the repo. One place to triage, one MCP target.
  In the current constellation: `dk1`.
- **Workspace owner** — per workspace, the single machine holding the canonical
  checkout: the default execution host and the sole author of that workspace's
  knowledge records. A declared binding, never an inference; recorded on the hub's
  workspace entry and mirrored locally at link time. Never selected per-task.
- **Execution host** — per task, the machine where the agent run is placed. Selected
  by the orchestrator at triage, like `crew`; defaults to the owner; requested and
  actual placement are snapshotted immutably on each run. Any host advertising a
  checkout can execute, because coordination writes go back over MCP. Applies to
  *shipped* runs only — shipping is opt-in, and a human can instead claim a task,
  work it in a local checkout, and resolve it via PR with no run or placement at
  all.
- **Run lease** — the hub→satellite protocol: satellite-placed runs wait in
  `placed`; each satellite polls `orbit.run.lease` (minute cadence, `runner`
  capability set) and executes what it leases. The poll doubles as the heartbeat.
  The hub is a mailbox, not a relay — it never connects to a satellite.
- **Data placement tiers** — per record type: *hub-only* (tasks, frictions),
  *owner-authored / hub-allocated IDs / Git-replicated* (learnings, ADRs), and
  *local-derived* (code graph, docs index, routine scheduler state).
- **Routine ownership rule** — the host pinned in a git-committed routine definition
  is the host in charge of that routine. Uncommitted (location-scoped local) routines
  are implicitly this-host.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Host identity file (`host.toml`) | [crates/orbit-remote/src/host_identity.rs](../../../crates/orbit-remote/src/host_identity.rs) | [ORB-10302], [ORB-10319] |
| Global/workspace seeding (`orbit init`) | [crates/orbit-cli/src/command/init.rs](../../../crates/orbit-cli/src/command/init.rs) | [ORB-10319] |
| Versioned logical-workspace catalog + local checkout bindings | [crates/orbit-remote/src/workspace_registry.rs](../../../crates/orbit-remote/src/workspace_registry.rs) | [ORB-10248], [ORB-10302], [ORB-10319] |
| Host/workspace registry service, persistence, profiles, and satellite cache | [crates/orbit-remote/src/](../../../crates/orbit-remote/src/) | [ORB-10302], [ORB-10319] |
| Logical task coordination registry + single-authority allocator | [crates/orbit-store/src/sqlite/task_registry/](../../../crates/orbit-store/src/sqlite/task_registry/) | [ORB-10249] |
| MCP composition, broker, hub trust/server/link, and registration | [crates/orbit-remote/src/mcp/](../../../crates/orbit-remote/src/mcp/) | [ORB-10262], [ORB-10268], [ORB-10269], [ORB-10271], [ORB-10319] |
| Generic MCP framing and raw client | [crates/orbit-mcp/](../../../crates/orbit-mcp/) | [ORB-10319] |
| Routine sweep host filter | [docs/design/routines/2_design.md](../routines/2_design.md) | — |

Detailed mechanisms in [2_design.md](./2_design.md); open directions in
[3_vision.md](./3_vision.md).

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification this registry
  complements (one canonical contract, one hub target, local owner/derived
  placement, Bridge parity retired).
- [ORB-10248] — split the versioned workspace catalog from machine-local
  checkout paths and owner/replica bindings.
- [ORB-10249] — split logical task-registry workspace records from optional
  local checkout bindings and made task relations/readiness global by task ID.
- [ORB-10268] — implemented the machine-global hub trust document and the
  checkoutless hub MCP endpoint that consumes registry identity and store authority.
- [ORB-10302] — established `orbit-registry` as the host/workspace domain crate,
  retaining execution-profile construction in `orbit-core` and persistence in
  `orbit-store` ([ADR-0235]).
- [ORB-10319] — replaces that horizontal boundary with the vertical
  `orbit-remote` feature crate: registry behavior, feature-owned SQLite access,
  profile/cache composition, MCP routing, hub/link, and registration now evolve
  together over neutral Store, MCP, Core, Tools, and Common kernels ([ADR-0240]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

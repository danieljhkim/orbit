---
title: Host Registry — Overview
owner: claude
last_updated: 2026-08-10
last_validated: 2026-08-10
status: Draft
feature: host-registry
doc_role: overview
type: design
summary: Every machine is its own coordination host for the workspaces it owns; a machine-scoped task-id prefix buys global uniqueness without a global allocator, and ownership is declared per workspace in the machine-local registry.
tags: [host-registry, multi-host, ownership, task-ids]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**", "crates/orbit-common/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access]
related_artifacts: [ORB-00424, ORB-10248, ORB-10249, ORB-10268, ORB-10302, ORB-10319, ORB-10332, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0352, ADR-0355, ADR-0356, ADR-0357, ADR-0358]
---

# Host Registry — Overview

Machine identity is a first-class Orbit concept: every machine carries a validated
identity (`host.toml`) and a task-id prefix chosen once, and every workspace
declares which machine owns it. **Every machine is its own coordination host for
the workspaces it owns** — there is no fleet-wide hub, and no machine-level role
to choose. A workspace you own coordinates locally; a workspace someone else owns
is reached by talking to that owner over the existing SSH-carried MCP route, and
is refused locally rather than half-served. Global uniqueness of task IDs comes
from partitioning the namespace by machine, not from routing every project through
one allocator.

> **Status: Draft — structural rewrite in flight.** This revision replaces the
> singular-hub model that [ADR-0226], [ADR-0229], and [ADR-0230] described and
> that [ORB-10268], [ORB-10271], and [ORB-10272] partially implemented. Sections
> marked **v2** are deliberately deferred, not designed. See
> [4_decisions.md](./4_decisions.md) for what was superseded and why.

## 1. Motivation

Orbit was single-machine by construction: `~/.orbit/orbit.db`, a single `local`
task-ID authority, and `workspaces.json` all implicitly described "this box." The
first cross-machine design fixed that by electing one **hub** and routing every
workspace's coordination records through it, with each machine declaring
`mode = standalone | hub | spoke` at init.

That model has a defect that only shows up once you own more than one machine.
`mode` is machine-level and set once, so a laptop that becomes a `spoke` routes
*every* workspace on it to the hub — including projects that exist only on that
laptop and have no business leaving it. The design says so plainly: coordination
records live on the hub "for every workspace regardless of who owns the repo."
The only way to keep a project local was to give up global task-ID uniqueness,
because uniqueness was a property of the single allocator.

Three corrections follow:

1. **Uniqueness is a namespace problem, not an authority problem.** A task-id
   prefix fixed per machine at global init (`ORB-`, `DE-`, …) makes IDs globally
   unique by partition. No allocator, no reconciliation, no activation
   transition. It also makes the *repair* case benign: if two machines ever
   disagree about who owns a workspace, their task sets are disjoint by
   construction and merge by union.
2. **Coordination authority is per workspace, not per machine.** One writer per
   workspace is the invariant that actually matters; "one writer globally" was a
   stronger claim than anything needed, and it is what forced local projects
   through the hub. A machine can coordinate some workspaces and hold
   read-only checkouts of others.
3. **Placement was solving a problem we don't have yet.** Cross-machine
   *execution* — the run queue, presence map, lease protocol, and runner
   capability — is a substantial subsystem whose only v1 consumer would be
   convenience. The cross-machine surface that is actually needed is narrower:
   create and read tasks on a machine that isn't this one. That already works
   over the client→hub MCP route.

The registry still does **not** introduce store synchronization ([ADR-0200], the
archived `_archive/task-sync/` design). Partition without replication.

## 2. Core Concepts

- **Host identity** — the machine-local declaration in `~/.orbit/host.toml`: a
  stable generated `machine_id`, a renameable human `host_id`, and the machine's
  `task_prefix`. Names appear only in human-authored text and resolve at binding
  time; everything the system persists stores `machine_id`. Initialized by
  `orbit init`. There is no `mode` field ([ADR-0355]).
- **Task prefix** — the machine-scoped namespace for every task ID that machine
  mints, chosen once at global init and immutable thereafter. Uniqueness across
  machines is a human-scale choice, not a coordinated allocation ([ADR-0356]).
- **Coordination host** — per workspace, the machine whose store holds that
  workspace's tasks. Every machine is the coordination host for the workspaces it
  owns; the degenerate case, a machine that owns everything it has, is what used
  to be called "standalone."
- **Workspace owner** — per workspace, the single machine holding the canonical
  checkout and the sole author of that workspace's coordination and knowledge
  records. A declared binding, never an inference. In v1, `coordination_host` and
  `owner` are the same machine; they are kept as separate concepts because
  splitting them later (canonical checkout here, triage queue there) is a
  legitimate configuration and costs nothing to leave open.
- **Local checkout, non-owned** — a checkout of a workspace this machine does not
  own. It stays present in the local registry so path resolution and the
  local-derived tier keep working, but it is hidden from `orbit workspace list`
  and refuses coordination writes, naming the owner in the error ([ADR-0355]).
- **Local workspace registry** — `workspaces.json`, the v1 source of truth for
  which machine owns what. Self-asserted and unverified by design; a registration
  protocol that arbitrates competing claims is v2.
- **Data placement tiers** — per record type: *owner-coordinated* (tasks),
  *workspace-scoped and git-carried* (learnings, ADRs, frictions — keyed
  `(workspace_id, artifact_key)`, never globally allocated, [ADR-0357]), and
  *local-derived* (code graph, docs index, routine scheduler state).
- **Routine ownership rule** — the host pinned in a git-committed routine
  definition is the host in charge of that routine. Uncommitted
  (location-scoped local) routines are implicitly this-host.

Deferred to **v2**, with no v1 substitute: host registration and the fleet
inventory, the workspace presence map, execution placement, run leases, and the
`runner` capability ([ADR-0358]).

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Host identity file (`host.toml`), incl. `task_prefix` | [crates/orbit-remote/src/host_identity.rs](../../../crates/orbit-remote/src/host_identity.rs) | [ORB-10302], [ORB-10319] |
| Global/workspace seeding (`orbit init`) | [crates/orbit-cli/src/command/init.rs](../../../crates/orbit-cli/src/command/init.rs) | [ORB-10319] |
| Versioned logical-workspace catalog + local checkout bindings and roles | [crates/orbit-remote/src/workspace_registry.rs](../../../crates/orbit-remote/src/workspace_registry.rs) | [ORB-10248], [ORB-10302], [ORB-10319] |
| Task coordination registry + per-machine prefixed allocator | [crates/orbit-store/src/sqlite/task_registry/](../../../crates/orbit-store/src/sqlite/task_registry/) | [ORB-10249] |
| MCP composition, broker, and client→owner link | [crates/orbit-remote/src/mcp/](../../../crates/orbit-remote/src/mcp/) | [ORB-10262], [ORB-10268], [ORB-10319] |
| Generic MCP framing and raw client | [crates/orbit-mcp/](../../../crates/orbit-mcp/) | [ORB-10319] |
| Routine sweep host filter | [docs/design/routines/2_design.md](../routines/2_design.md) | — |

Detailed mechanisms in [2_design.md](./2_design.md); deferred work and open
questions in [3_vision.md](./3_vision.md).

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification this design
  complements (one canonical contract, local owner/derived placement, Bridge
  parity retired).
- [ORB-10248] — split the versioned workspace catalog from machine-local
  checkout paths and owner/replica bindings. This split is what lets v1 record
  ownership per workspace without a fleet registry.
- [ORB-10249] — split logical task-registry workspace records from optional
  local checkout bindings and made task relations/readiness global by task ID.
- [ORB-10268] — implemented the machine-global hub trust document and the
  checkoutless hub MCP endpoint. The trust document survives as the client's
  per-route policy; the singular-hub assumption around it does not.
- [ORB-10302] — established `orbit-registry` as the host/workspace domain crate.
- [ORB-10319] — replaced that horizontal boundary with the vertical
  `orbit-remote` feature crate ([ADR-0240]). Unaffected by this revision.
- [ORB-10332] — removed the `orbit.host.list` MCP discovery tool as unused.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

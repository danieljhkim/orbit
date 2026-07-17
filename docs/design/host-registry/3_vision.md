---
title: Host Registry — Vision
owner: claude
last_updated: 2026-07-16
status: Draft
feature: host-registry
doc_role: vision
type: design
summary: Open questions (non-owner knowledge authoring, label taxonomy, ownership migration, coordination-plane distribution) and prior art (CI runner registration, cluster membership, the rejected task-sync design) for the host registry.
tags: [host-registry, multi-host, dispatch]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access]
related_artifacts: [ORB-00424]
---

# Host Registry — Vision

Forward-looking notes for the host registry. Everything here is unbuilt and most of
it is deliberately deferred; the v1 posture is the smallest registry that makes
placement validated, leased, and auditable, and dead pins visible. Speculation is
labelled as such.

## 1. Open Questions

1. **Non-owner knowledge authoring.** V1 makes learnings/ADRs strictly one-writer
   per workspace — the owner authors locally with a hub-allocated ID
   ([2_design.md §5](./2_design.md)), and a non-owner routes anything actionable as
   a task to the owner. Letting a non-owner author directly requires either a
   reservation/finalization protocol (expiry, orphaned IDs, finalize/pull races) or
   hub-relayed content writes. Design it only if the route-through-owner seam
   demonstrably hurts.
2. **Label taxonomy.** Free-form labels suffice for one operator who names things
   consistently. Does placement ever need a structured capability schema (provider
   versions, GPU, checkout freshness), or is that the over-engineering the crew
   design avoided by choosing `description` + `tags`?
3. **Ownership migration.** With coordination fixed at the hub, moving a
   workspace's owner is lighter than full authority migration would have been:
   ensure a checkout, flip both sides of the binding, reindex knowledge from files.
   Still: supported `orbit host` operation or documented manual runbook?
   (Speculation: runbook first; tooling only after it has been done twice.)
4. **Distributing the coordination plane.** Ownership already distributes; the
   hub does not. If the constellation ever outgrows one coordination host, does the
   plane shard per-owner (each owner coordinates its own workspaces — reintroducing
   multi-target orchestration) or replicate (reintroducing everything [ADR-0200]
   rejected)? Current answer: neither; the hub is small enough to keep singular.
5. **Runner credential lifecycle.** Satellites hold standing SSH identities with
   `runner` capability. Rotation, revocation on retire, and scoping per satellite
   are manual in v1; at what host count does that need tooling?
6. **Poll cadence and backoff.** One minute matches the sweep and bounds placement
   latency acceptably today. Do busy or battery-powered satellites need adaptive
   cadence, and does the lease TTL interact with it?
7. **Should placement ever auto-fail-over?** V1 returns an expired lease to
   `placed` for the shepherd to re-place explicitly. Automatic re-placement to
   another labelled host is attractive and is exactly how blind dispatch re-enters
   the system; the triage policy (orchestrator-steered, explicit ids) argues
   against it.

## 2. Prior Work

### CI runner registration

GitHub Actions self-hosted runners are the closest shape: machines register with a
central coordinator, advertise labels, poll for work, and take leases; dead runners
are visible in the inventory. The design borrows this wholesale — registration,
labels, last-seen, *and* the pull-based lease. The deliberate difference: no
queue-side auto-assignment — placement stays an orchestrator decision per run.

### Cluster membership

Nomad clients and Kubernetes nodes register with fingerprinted capabilities and
heartbeat to a scheduler. Serf/memberlist solve liveness with gossip. Orbit wants
the *inventory* half, not the *scheduler* half: one operator, single-digit hosts,
and SSH as the only transport make gossip, leader election, and lease protocols
beyond a simple TTL wildly oversized.

### Orbit-internal

- [remote-access/4_decisions.md](../remote-access/4_decisions.md) — [ADR-0200]
  rejected the git-orphan-branch task registry in favor of read-only viewing;
  [ADR-0201] fixed the SSH-only, no-network-bind posture this design inherits.
- `docs/design/_archive/task-sync/` — the fullest prior exploration of
  cross-machine task state; its failure modes (ID collision, merge semantics) are
  why tasks are MCP-only here rather than synced.
- [routines/2_design.md](../routines/2_design.md) — explicit host pinning,
  host-local scheduler state ([ADR-0208]), the load-time name-collision error, and
  the minute-cadence clock unit the runner poll rides on.
- [mcp-bridge/2_design.md](../mcp-bridge/2_design.md) ([ORB-00424]) — the
  placement-aware local broker, client→hub transport, capability sets, and audit
  context; this registry supplies stable identity, ownership bindings, and the
  hub→satellite lease half.

## 3. What May Be Distinctive

Possibly nothing algorithmically — registration, labels, leases, and freshness are
standard runner machinery. The distinctive posture is *what is refused*: no network
listener, no token auth, no scheduler, no auto-assignment, no machine-to-machine
routes (the hub is a mailbox, not a proxy), and no daemon beyond the clock unit
that already exists — identity and work distribution ride an existing SSH
relationship at sweep cadence, and every placement decision stays with the
orchestrator. Placement without replication; inventory without a control plane.

## 4. References

**Orbit-internal**

- [1_overview.md](./1_overview.md), [2_design.md](./2_design.md)
- [mcp-bridge/1_overview.md](../mcp-bridge/1_overview.md)
- [remote-access/1_overview.md](../remote-access/1_overview.md)
- [routines/2_design.md](../routines/2_design.md)

**External**

- GitHub Actions self-hosted runners — registration, labels, polling, runner groups.
- HashiCorp Nomad client fingerprinting; Kubernetes node registration/heartbeats.
- HashiCorp Serf / memberlist — gossip-based membership (considered, oversized).

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification this registry
  complements.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

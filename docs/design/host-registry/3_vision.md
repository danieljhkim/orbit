---
title: Host Registry — Vision
owner: claude
last_updated: 2026-08-10
last_validated: 2026-08-10
status: Draft
feature: host-registry
doc_role: vision
type: design
summary: Deferred v2 work (workspace registration, prefix arbitration, execution placement) and prior art (CI runner registration, cluster membership, the rejected task-sync design) for the per-machine coordination model.
tags: [host-registry, multi-host, ownership]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access]
related_artifacts: [ORB-00424, ORB-10319]
---

# Host Registry — Vision

Forward-looking notes for the per-machine coordination model. The v1 posture is
deliberately the smallest thing that keeps projects local and task IDs unique:
ownership declared in each machine's own registry, uniqueness by prefix partition,
and no fleet inventory at all. Most of what follows is work that was *built and
then deferred* rather than work never attempted — see
[2_design.md §2.1](./2_design.md) for what is dormant in the tree.

These questions assume the vertical boundary from [ORB-10319] / [Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate):
`orbit-remote` owns the feature from persistence through MCP composition, while
Store and MCP remain neutral infrastructure. A future transport or availability
design should extend that feature boundary rather than redistribute its policy
across kernels.

## 1. Open Questions

1. **Workspace registration (the main v2 question).** V1 lets every machine assert
   ownership with nothing arbitrating. Registration would give a shared inventory,
   a join-time duplicate check, and an answer to "who owns this?" that does not
   depend on asking each machine in turn. The shipped registry core
   ([2_design.md §2.1](./2_design.md)) is most of the mechanism. The open part is
   what registration *means* when there is no longer a single hub: registering with
   whom? Plausible answer — one machine volunteers as directory, holding names and
   claims but no coordination records, which is a much smaller thing than the hub
   this design just retired. Design it when a second person or a third machine
   makes self-assertion actually hurt.
2. **Prefix arbitration and collision detection.** Related but separable: even
   without a directory, a cheap check would help — a lint that compares prefixes
   across the routes in `mcp.toml`, or a warning when a merged search returns two
   records whose IDs collide. Worth doing before registration, since it is a fraction
   of the work and catches the failure v1 leaves silent.
3. **Execution placement.** Withdrawn in v1 ([Defer fleet registration and execution placement to v2](./4_decisions.md#defer-fleet-registration-and-execution-placement-to-v2)), with the design intact in
   git history: pull-based leases, immutable requested/actual placement snapshots,
   a `runner` capability, and the presence map that validated a target host
   advertises a checkout. Revive it when there is a real case for running a task
   somewhere other than where its workspace lives — a machine with a GPU, a
   provider only installed on one box, or a laptop that should offload. Note the
   direction constraint: this needs the owner to reach *out*, or the executor to
   poll, and v1 has neither.
4. **Splitting coordination host from owner.** V1 forces them equal. The
   configuration they would allow — canonical checkout on a laptop, triage queue on
   a server — is legitimate and was expressible in the superseded model. Keeping
   them as two concepts costs nothing today; making them two *fields* costs a
   routing decision on every coordination write. Do it when someone wants it, not
   before.
5. **Ownership migration.** Moving a workspace between owners is now a row copy
   plus a binding flip, and crucially needs no renumbering: prefixes travel with
   the records. Supported `orbit workspace transfer` operation or documented
   runbook? (Speculation: runbook first; tooling after it has been done twice.)
6. **Non-owner knowledge authoring.** Still one-writer per workspace, and now with
   no allocator the reservation protocol that made this hard has evaporated. What
   remains is the merge question: two machines writing `(workspace_id,
   artifact_key)` records to the same repo is a git conflict, not a corruption.
   That may be tolerable enough to just allow. Revisit if the
   route-through-owner seam demonstrably hurts.
7. **Label taxonomy.** Free-form labels suffice for one operator who names things
   consistently. If placement returns, does it need a structured capability schema
   (provider versions, GPU, checkout freshness), or is that the over-engineering the
   crew design avoided by choosing `description` + `tags`?
8. **Liveness, if it ever matters.** V1 has no `last_seen` and no heartbeat: you
   discover a machine is down by calling it. Anything that wants to *pre-empt* that
   — a dashboard, a router, a scheduler — needs a heartbeat, which needs
   registration, which is question 1.

## 2. Prior Work

### CI runner registration

GitHub Actions self-hosted runners are the closest shape to what v1 *rejected*:
machines register with a central coordinator, advertise labels, poll for work, and
take leases. The superseded design borrowed it wholesale. The lesson taken from
withdrawing it is that the runner model answers a question — distributing execution
across a fleet — that a single-operator constellation does not yet ask, and its
machinery (registration, presence, leases, standing credentials) is nearly all
in service of that one question. It remains the right reference if question 3
returns.

### Cluster membership

Nomad clients and Kubernetes nodes register with fingerprinted capabilities and
heartbeat to a scheduler. Serf/memberlist solve liveness with gossip. Orbit wants
neither half at present: one operator and single-digit hosts make gossip, leader
election, and lease protocols wildly oversized, and v1 goes further than the
earlier draft by declining the *inventory* half too.

### Project-key namespacing

Jira project keys and Linear team prefixes solve exactly the problem `task_prefix`
solves: globally-unique-looking identifiers with no global allocator, partitioned
by a short human-chosen string that is immutable in practice because it leaks into
everything. Both also demonstrate the failure mode v1 accepts — key collisions
across independent instances are a known migration headache, handled at import
time rather than prevented.

### Orbit-internal

- [remote-access/4_decisions.md](../remote-access/4_decisions.md) — [Live remote/multi-workspace dashboard viewing supersedes the git-sync task registry](../remote-access/4_decisions.md#live-remotemulti-workspace-dashboard-viewing-supersedes-the-git-sync-task-registry)
  rejected the git-orphan-branch task registry in favor of read-only viewing;
  [Remote dashboard access is an SSH tunnel over a loopback-only bind, never a network bind with auth](../remote-access/4_decisions.md#remote-dashboard-access-is-an-ssh-tunnel-over-a-loopback-only-bind-never-a-network-bind-with-auth) fixed the SSH-only, no-network-bind posture this design inherits.
- `docs/design/_archive/task-sync/` — the fullest prior exploration of
  cross-machine task state; its failure modes (ID collision, merge semantics) are
  why tasks are MCP-only here rather than synced.
- [routines/2_design.md](../routines/2_design.md) — explicit host pinning,
  host-local scheduler state ([Routine definitions are git-shared; scheduler state is host-local and never synced](../routines/4_decisions.md#routine-definitions-are-git-shared-scheduler-state-is-host-local-and-never-synced)), and the load-time name-collision error.
- [mcp-bridge/2_design.md](../mcp-bridge/2_design.md) ([ORB-00424]) — the
  placement-aware local broker, client→owner transport, capability sets, and audit
  context; this feature supplies stable identity and ownership bindings.

## 3. What May Be Distinctive

Nothing algorithmically — per-machine sequences and human-chosen key prefixes are
old ideas, and the withdrawn runner model was standard machinery. The distinctive
posture is *what is refused*: no fleet inventory, no registration, no global
allocator, no scheduler, no network listener, no token auth, no heartbeat, and no
machine that must be up for another machine's local project to work. Uniqueness
without an authority; multi-machine without a control plane. The bet is that a
constellation of single-digit machines run by one person needs coordination scoped
to a workspace and nothing wider, and that the correct amount of fleet
infrastructure at this size is zero.

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
- Jira project keys / Linear team prefixes — namespace partitioning as a substitute
  for a global allocator, and the import-time collision handling it implies.

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification this registry
  complements.
- [ORB-10319] — consolidates the registry and MCP bridge into the vertical
  `orbit-remote` feature boundary assumed by these open questions.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

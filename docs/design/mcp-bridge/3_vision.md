---
title: Orbit MCP Bridge — Vision
owner: codex
last_updated: 2026-08-09
last_validated: 2026-08-02
status: Accepted
feature: mcp-bridge
doc_role: vision
type: design
summary: Open questions and prior art for a singular hub MCP link, owner-bound knowledge, explicit Git-replica reads, execution-profile projection, schema skew, host identity assurance, and what surface the owned tunnel should carry.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access, orbit-search]
related_artifacts: [ORB-00424, ORB-10319, ADR-0181, ADR-0199, ADR-0200, ADR-0201, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0240, ADR-0350, ADR-0351]
---

# Orbit MCP Bridge — Vision

Forward-looking notes for the MCP bridge. V1 is deliberately narrow: one trusted
operator, one coordination hub, a small host fleet, SSH transport, one declared
owner per workspace, no spoke-to-spoke routes, and no offline coordination writes.
Speculation below should not leak into implementation without demonstrated need.
It also assumes [ADR-0240]'s vertical `orbit-remote` owner over neutral MCP, Store,
Core, Tools, and Common kernels; new remote behavior should normally extend that
feature rather than add another horizontal broker crate ([ORB-10319]).

## 1. Open Questions

1. **Agent non-owner knowledge authoring.** The coupled v1 answer is "route a task
   to the owner." The human manual path may allocate an ID and carry a narrative
   file through a PR, but it does not enable replica-store writes. If the agent seam
   hurts, should the hub accept content and queue an owner finalize action, should
   IDs gain a real reservation/finalization lifecycle, or should ownership rules
   change? Any option broadens the hub beyond mailbox/coordination metadata.
2. **Replica freshness UX.** What proves a Git knowledge replica is suitable for an
   explicit read: indexed commit equals checkout HEAD, branch matches owner default,
   a maximum age, or a signed owner projection? V1 should expose facts rather than
   claim freshness it cannot know.
3. **Owner execution-profile freshness.** Crew/dispatch validation requires a
   one-way owner→hub projection. Is poll-time publication enough, or should config
   changes trigger an immediate publish? What fields belong in the profile without
   turning it into a copy of `.orbit/config.toml`?
4. **Authenticated caller-machine identity.** V1 trusts caller host as provenance
   inside a same-user SSH fleet. If hosts become mutually untrusted, should
   registration bind a dedicated SSH key/principal to `machine_id`, or should Orbit
   sign a session challenge? This must be solved before machine identity becomes an
   authorization boundary.
5. **Contract skew policy.** Is one MCP contract revision enough, or should hub and
   local subsets carry separate schema hashes so graph-only local changes do not
   block coordination calls? Start coarse; split only after real mixed-version
   deployments create friction.
6. **Transport beyond SSH.** Narrowed by [ADR-0350]. The owned tunnel adds a
   loopback listener while keeping SSH as the authenticator, so listener hardening
   is a bind guard rather than an authentication system, and Orbit still owns no
   credential. What remains open is the original case: genuinely shell-less
   environments, where Streamable HTTP would require Orbit to own authentication
   and session management outright. Is there such a deployment?
7. **Distributing the coordination plane.** Host-registry currently says no. If the
   hub ever shards or replicates, the MCP bridge loses its one-target invariant and
   needs a new design rather than a configurable list of hidden authorities.
8. **Knowledge sidecar on replicas.** Should an agent explicitly opt into stale but
   marked learning injection, or is disabling injection safer until owner-current
   state is available? This should follow evidence from replica use, not convenience.
9. **Whether the advertised per-tool surface should become generic dispatch.**
   [ADR-0351] adds command and changes nothing else, leaving this open. The
   replacement shape would be two operations — enumerate the registry entries
   visible to a caller with their schemas, and invoke one by name — collapsing
   per-tool policy into a single authorization point. Note the argument is *not*
   that the definitions are expensive to maintain; they are generated from the
   registry. It is whether policy and placement metadata, the conformance count,
   the contract digest, and the per-request context cost of carrying every schema
   are worth it once one authorization point exists.

   Two things must be settled before it could land. It rebuilds the transport's
   own list and call verbs inside a tool, which needs arguing on its merits rather
   than assuming. And capability filtering currently happens twice — at
   advertisement and at call — so a generic invoke inherits the whole burden:
   tools registered inactive or active-without-policy (`orbit.adr.list`,
   `orbit.task.delete`, the pipeline primitives) are unreachable over MCP today
   *only* because they never appear in the advertised set. Generic invoke must
   deny them explicitly, or the replacement quietly grants an agent
   `orbit.task.delete` while nothing appears to have changed.

   Deciding it needs evidence no endpoint currently produces — see the audit-event
   cut described in [2_design.md §5.3](./2_design.md).

## 2. Prior Work

### Orbit MCP and session context

The generic MCP adapter separates protocol framing from an injected `McpHost`,
sanitizes advertised names, resolves structural schemas, and threads
`ToolSessionContext` into dispatch. `orbit-remote` composes generic builtin schemas
with Remote-owned discovery/graph definitions and supplies the in-process graph,
learning, hub, and broker behavior. [ADR-0181] established deliberate workspace
context instead of cwd fallback; [ADR-0199] proposed per-call runtime resolution.
The local broker extends those neutral seams rather than starting a second
implementation.

### Host registry and star topology

[host-registry/2_design.md](../host-registry/2_design.md) supplies stable machine
identity, a singular hub, declared workspace ownership, local replica role,
presence maps, requested/actual placement, and pull-based run leases. The MCP bridge
consumes those facts and never becomes a scheduler or owner proxy.

### Remote access

[remote-access/4_decisions.md](../remote-access/4_decisions.md) established two
constraints reused here: do not synchronize task stores, and use SSH over a local-
only process instead of adding a routable unauthenticated service. MCP adds writable
coordination while preserving that transport posture.

### Bridge parity

Bridge's parity layer proved the need for off-box task/workflow access and
multi-workspace selection. It also proved the maintenance cost of copying schemas
and translating through a dashboard projection. The migration preserves the need
and removes the duplicate contract.

### Search partitioning

Orbit search already ranks within each kind and round-robins task, doc, ADR, and
learning branches under a total limit. Role-aware composition moves branches to hub,
owner/local, or explicit replica sources without inventing another relevance model.

### External patterns

- SSH stdio is a common transport for Git, remote language servers, and MCP servers:
  SSH owns identity/encryption while the application owns framing.
- CI runner systems separate a central queue from machines that poll and execute.
  Orbit borrows the pull direction but keeps placement orchestrator-selected.
- Git's distributed read model is the relevant precedent for owner-authored
  knowledge: replicas are useful and inspectable, but not automatically current.

## 3. What May Be Distinctive

The distinctive part is the refusal to make "one MCP surface" mean "one machine
serves every datum." The local broker has one network destination, yet placement
still follows record semantics: coordination goes to the hub, current knowledge
stays with its owner, derived indexes stay with the checkout, and composite tools
state exactly which pieces are unavailable. The hub is a mailbox and allocator, not
a relay or universal read proxy.

## 4. References

**Orbit-internal**

- [1_overview.md](./1_overview.md), [2_design.md](./2_design.md)
- [host-registry/1_overview.md](../host-registry/1_overview.md)
- [mcp-session-context/2_design.md](../mcp-session-context/2_design.md)
- [remote-access/2_design.md](../remote-access/2_design.md)
- [orbit-search/2_design.md](../orbit-search/2_design.md)
- [archived Orbit Graph design](../_archive/orbit-graph/2_design.md)

**External**

- Model Context Protocol — initialization metadata, tool discovery, stdio transport.
- OpenSSH — host aliases, key authentication, host verification, stdio process
  transport.
- GitHub Actions self-hosted runners — central queue with pull-based execution.

## Task References

- [ORB-00424] — umbrella proposal for canonical Orbit MCP and Bridge parity
  retirement.
- [ORB-10319] — consolidates registry persistence and MCP routing in the vertical
  `orbit-remote` feature boundary assumed here ([ADR-0240]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

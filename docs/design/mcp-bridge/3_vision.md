---
title: Orbit MCP Bridge — Vision
owner: claude
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: mcp-bridge
doc_role: vision
type: design
summary: Open questions and prior art for the owner-machine MCP route, schema skew, host identity assurance, transport evolution, and whether the advertised tool surface should become generic dispatch.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access, orbit-search]
related_artifacts: [ORB-00424, ORB-10319, ORB-10736, ORB-10767, ORB-10768, ADR-0181, ADR-0199, ADR-0200, ADR-0201, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0240, ADR-0350, ADR-0351, ADR-0355, ADR-0356, ADR-0357, ADR-0358, ADR-0359]
---

# Orbit MCP Bridge — Vision

> **Learning-subsystem retirement.** [ORB-10736] / [ADR-0359] removed the native
> project-learning resource. Its authoring, replica, sidecar, and search questions
> are closed by removal rather than carried as future MCP work.

> **Status: Draft — structural rewrite landed.** The singular-hub contract
> ([ADR-0226], [ADR-0229], [ADR-0230]) is superseded by [ADR-0355]–[ADR-0358],
> recorded in [../host-registry/4_decisions.md](../host-registry/4_decisions.md).
> Questions and prior art below that concern execution placement, run leases, and
> host registration are **deferred to v2**.

Forward-looking notes for the MCP bridge. V1 is deliberately narrow: one trusted
operator, per-machine coordination for the workspaces each machine owns, a small
host fleet, SSH transport, one declared owner per workspace, and no offline
coordination writes for workspaces this machine does not own. Speculation below
should not leak into implementation without demonstrated need. It also assumes
[ADR-0240]'s vertical `orbit-remote` owner over neutral MCP, Store, Core, Tools,
and Common kernels; new remote behavior should normally extend that feature rather
than add another horizontal broker crate ([ORB-10319]).

## 1. Open Questions

1. **Owner execution-profile freshness (v2).** Deferred with execution placement
   ([ADR-0358]). If cross-machine dispatch returns, crew validation would again need
   a one-way owner→coordinator projection, and the questions are unchanged: is
   poll-time publication enough, or should config changes trigger an immediate
   publish? What fields belong in the profile without turning it into a copy of
   `.orbit/config.toml`? In v1 crew validation reads the owner machine's local
   config directly, so none of this is live.
2. **Authenticated caller-machine identity.** V1 trusts caller host as provenance
   inside a same-user SSH fleet. If hosts become mutually untrusted, should a v2
   registration bind a dedicated SSH key/principal to `machine_id`, or should Orbit
   sign a session challenge? This must be solved before machine identity becomes an
   authorization boundary.
3. **Contract skew policy.** Is one MCP contract revision enough, or should the
   owner-routed and local subsets carry separate schema hashes so graph-only local
   changes do not block coordination calls? Start coarse; split only after real
   mixed-version deployments create friction.
4. **Transport beyond SSH.** Narrowed by [ADR-0350]. The owned tunnel adds a
   loopback listener while keeping SSH as the authenticator, so listener hardening
   is a bind guard rather than an authentication system, and Orbit still owns no
   credential. What remains open is the original case: genuinely shell-less
   environments, where Streamable HTTP would require Orbit to own authentication
   and session management outright. Is there such a deployment?
5. **Distributing the coordination plane — resolved.** The plane is now per-machine
   by construction ([ADR-0355]); there is no single target left to shard. What
   replaces the question is narrower: v1 exposes only the advertised task family across
   machines, so what, if anything, should cross next — friction triage or workflow
   observation — and does any of it justify a machine
   answering for a workspace it does not own? That is a v2 question and should be
   answered per record type, not as a topology change.
6. **Whether the advertised per-tool surface should become generic dispatch.**
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
   tools registered inactive or active-without-policy (`orbit.task.delete`, the
   pipeline primitives) are unreachable over MCP today *only* because they never
   appear in the advertised set. Generic invoke must deny them explicitly, or the
   replacement quietly grants an agent `orbit.task.delete` while nothing appears
   to have changed.

   Deciding it needs evidence no endpoint currently produces — see the audit-event
   cut described in [2_design.md §5.3](./2_design.md).

## 2. Prior Work

### Orbit MCP and session context

The generic MCP adapter separates protocol framing from an injected `McpHost`,
sanitizes advertised names, resolves structural schemas, and threads
`ToolSessionContext` into dispatch. `orbit-remote` composes generic builtin schemas
with Remote-owned discovery definitions and supplies coordination and broker
behavior. [ADR-0181] established deliberate
workspace context instead of cwd fallback; [ADR-0199] proposed per-call runtime
resolution. The local broker extends those neutral seams rather than starting a
second implementation.

### Host registry and per-machine coordination

[host-registry/2_design.md](../host-registry/2_design.md) supplies stable machine
identity, the machine-scoped task-id prefix, and declared per-workspace ownership
in the machine-local registry. The MCP bridge consumes those facts and never
becomes a scheduler or owner proxy. The fleet inventory, presence map,
requested/actual placement, and pull-based run leases that earlier drafts consumed
are deferred to v2 ([ADR-0358]).

### Remote access

[remote-access/4_decisions.md](../remote-access/4_decisions.md) established two
constraints reused here: do not synchronize task stores, and use SSH over a local-
only process instead of adding a routable unauthenticated service. MCP adds writable
coordination while preserving that transport posture.

### Bridge parity

Bridge's parity layer proved the need for task/workflow access and multi-workspace
selection. It also proved the maintenance cost of copying schemas and translating
through a dashboard projection. [ORB-10768] retired the service entirely once the
actual on-box clients registered Orbit directly; [ORB-10767] deliberately dropped
its worker invocation family and descoped `repo_sync`.

### Search partitioning

Orbit search ranks within each kind and round-robins task, doc, and friction
branches under a total limit. The current composite MCP placement executes the
whole query in one locally owned checkout; per-branch routing remains unimplemented
and is not planned by this design.

### External patterns

- SSH stdio is a common transport for Git, remote language servers, and MCP servers:
  SSH owns identity/encryption while the application owns framing.

### External patterns held for v2

- CI runner systems separate a central queue from machines that poll and execute.
  That is the shape a returning placement design would borrow — pull direction with
  orchestrator-selected placement — and it has no v1 consumer ([ADR-0358]).

## 3. What May Be Distinctive

The distinctive part is the refusal to make "one MCP surface" mean "one machine
serves every datum." The local broker has at most one network destination, yet
placement still follows record semantics: coordination goes to the machine that
owns the record, derived indexes stay with the checkout, and composite search
fails unless its whole local-runtime requirement is met. No machine is a relay or
a universal read proxy.

## 4. References

**Orbit-internal**

- [1_overview.md](./1_overview.md), [2_design.md](./2_design.md)
- [host-registry/1_overview.md](../host-registry/1_overview.md)
- [host-registry/4_decisions.md](../host-registry/4_decisions.md)
- [mcp-session-context/2_design.md](../mcp-session-context/2_design.md)
- [remote-access/2_design.md](../remote-access/2_design.md)
- [orbit-search/2_design.md](../orbit-search/2_design.md)
- [archived Orbit Graph design](../_archive/orbit-graph/2_design.md)

**External**

- Model Context Protocol — initialization metadata, tool discovery, stdio transport.
- OpenSSH — host aliases, key authentication, host verification, stdio process
  transport.
- GitHub Actions self-hosted runners — central queue with pull-based execution;
  prior art for the v2 placement question only.

## Task References

- [ORB-00424] — umbrella proposal for canonical Orbit MCP and Bridge parity
  retirement.
- [ORB-10319] — consolidates registry persistence and MCP routing in the vertical
  `orbit-remote` feature boundary assumed here ([ADR-0240]).
- [ORB-10736] — closed the learning authoring/replica/search questions by removing
  the native resource ([ADR-0359]).
- [ORB-10767] — deliberately dropped Bridge's worker invocation family and
  descoped `repo_sync`.
- [ORB-10768] — retired Bridge entirely after direct local Orbit registration.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

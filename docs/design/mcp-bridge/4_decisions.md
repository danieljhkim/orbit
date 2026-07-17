---
title: Orbit MCP Bridge — Decisions
owner: codex
last_updated: 2026-07-16
status: Accepted
feature: mcp-bridge
doc_role: decisions
type: design
summary: ADR log for the MCP bridge feature; no ADRs are allocated yet and candidate hub/owner decisions await coupled design acceptance.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-mcp/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool.rs"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access]
related_artifacts: [ORB-00424, ADR-0181, ADR-0199, ADR-0200, ADR-0201]
---

# Orbit MCP Bridge — Decisions

ADR log for `mcp-bridge`. Entries are append-only and ordered by ascending global
ID. **Allocate the global `ADR-NNNN` via `orbit.adr.add` before writing the
heading** — never hand-author a four-digit number. The store owns ID, status, owner,
and links; this file is the long-form narrative keyed on that same ID. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict).

**No ADRs allocated yet.** This feature design is Accepted; coupled
[host-registry](../host-registry/4_decisions.md) remains Draft. After both designs
are accepted, these candidates appear to clear the real-alternative, forward-
constraint, and non-trivial-cost bar:

1. **Orbit MCP has exactly one cross-machine target: the coordination hub.**
   Alternative: per-workspace authority/owner routes. Constraint: no spoke-to-spoke
   connections and no hub proxy to owners. Cost: current knowledge for spoke-owned
   workspaces is unavailable off-owner except as a Git replica.
2. **The client-facing MCP process is a local placement broker, not a whole-surface
   SSH relay.** Alternative: run every tool on the hub. Constraint: tools declare
   `hub`, `owner`, `local-derived`, or `composite`. Cost: Orbit owns routing,
   composite operations, and split audit even though there is only one remote link.
3. **Knowledge is owner-bound; the hub allocates IDs but does not finalize or proxy
   for spoke owners.** Alternative: all knowledge on the hub or a distributed
   reservation/finalization protocol. Constraint: an agent non-owner routes
   authoring as a task to the owner; the explicit human ID-plus-PR path does not
   enable replica-store writes. Cost: allocation/finalize failure may consume an
   unused ID, and non-owner current reads do not exist.
4. **`mcp.toml` grants transport trust to one stable hub `machine_id` only.**
   Alternative: default/per-workspace targets or transport fields in workspace
   bindings. Constraint: repo config cannot redirect/elevate, and ownership changes
   never change MCP transport; the first registration pins an out-of-band-copied
   hub ID rather than silently trusting the reached process. Cost: the operator must
   transfer the hub ID during bootstrap, and host-registry/`mcp.toml` drift needs
   explicit diagnostics.
5. **Replica knowledge reads are explicit and marked.** Alternative: silently use
   the local Git checkout whenever current owner state is unreachable. Constraint:
   search/show never present replica data as current; `kind=all` requires current,
   explicit replica, or explicit omission. Cost: common cross-machine searches may
   require an extra consistency choice.
6. **Capability and placement are orthogonal.** Alternative: assume hub tools are
   operator-only and local tools are agent-safe. Constraint: `agent`, `operator`,
   and `runner` filter independently of placement. Cost: conformance coverage spans
   capability × placement combinations.
7. **Caller-host and process-host provenance are distinct.** Alternative: retain
   the executing process hostname only. Constraint: nested hub sessions carry
   stable caller `machine_id`, and composite knowledge calls correlate hub/local
   audit. Cost: v1 same-user SSH treats caller identity as trusted provenance, not
   independently authenticated authorization.
8. **Owners publish dispatch projections to the hub.** Alternative: the hub reads
   owner files live or contacts owners during dispatch. Constraint: crew/workflow
   validation uses one-way published metadata and fails when stale. Cost: another
   projection lifecycle must be refreshed and diagnosed.
9. **Bridge retires all Orbit-shaped parity and workflow declarations.**
   Alternative: retain Bridge permanently as the remote compatibility layer.
   Constraint: Orbit alone owns `orbit_*` schemas and errors. Cost: clients keep two
   MCP registrations for Orbit and non-Orbit constellation services.

## Task References

- [ORB-00424] — umbrella proposal for canonical Orbit MCP and eventual Bridge
  parity retirement.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

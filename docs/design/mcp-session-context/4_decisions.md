---
summary: "MCP Session Context — Decisions"
type: design
title: "MCP Session Context — Decisions"
owner: codex
last_updated: 2026-08-01
status: Accepted
feature: mcp-session-context
doc_role: decisions
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-remote/src/mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-00406", "ORB-10228", "ORB-10262", "ORB-10319", "ORB-10448", "ADR-0181", "ADR-0199", "ADR-0149"]
---

# MCP Session Context — Decisions

ADR log for MCP session context. Format follows [docs/design/CONVENTIONS.md §4](../CONVENTIONS.md): each entry is `Context · Decision · Consequences`, every entry names at least one Cost, and numbers are append-only.

Historical note ([ORB-10479]): the entries listed below already held a global ADR allocation, but their store bodies were lost when the worktrees that authored them were reaped (see [F2026-07-163]). The narratives were restored into the store at their existing IDs — no ID was reallocated — and their headings reduced to pointer form. Restored here: [ADR-0181], [ADR-0199].

---

## ADR-0181 — MCP ambient workspace session context

**Status:** Accepted · 2026-05 · [ORB-00256], amended 2026-07 · [ORB-10228], selector advertised 2026-07 · [ORB-10448]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0181"}'`.

## ADR-0199 — Workspace_path-addressable MCP host tools with surface-scoped containment

**Status:** Accepted · 2026-07 · [ORB-00406], implemented by [ORB-10262], coordination partition key corrected 2026-07 · [ORB-10448]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0199"}'`.

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.
- [ORB-00406] proposes workspace_path-addressable host tools ([ADR-0199]).
- [ORB-10228] accepted and implemented the trusted-provenance amendment to [ADR-0181].
- [ORB-10262] accepted and implemented ADR-0199 through the exact-checkout local broker.
- [ORB-10319] consolidated the broker/session implementation in `orbit-remote`; it does not change ADR-0181 or ADR-0199 semantics.
- [ORB-10448] made both ADRs reachable from a managed worktree activity: the `workspace` selector is now advertised on every workspace-scoped tool, and hub-placement coordination reads address the checkout-identity partition. Neither changes ADR-0181 or ADR-0199 semantics; see [2_design.md §3a–3b](./2_design.md). The advertised-selector contract is a breaking `tools/list` schema change (RELEASING.md) and may warrant its own allocated ADR — this task's activity was not granted `orbit.adr.add`, so no global ID was allocated.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

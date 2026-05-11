## Context
The MCP safe list exposed the original eight read-only graph tools, but `orbit.graph.history` later joined the registered graph tool surface as a read-only history/query tool. That left Codex and Gemini MCP discovery dependent on a stale client-visible safe list even though the runtime registry had the tool.

## Decision
Treat the MCP graph surface as the full read-only graph tool set: callers, deps, history, implementors, overview, pack, refs, search, and show. Continue excluding graph mutation tools (`add`, `delete`, `move`, `write`) from the public MCP surface.

## Consequences
- Codex, Gemini, and other MCP clients discover the same read-only graph capabilities that `orbit tool list` reports as active.
- The history tool is now a compatibility stub after ADR-029; agents should use `git log --grep '[T<task-id>]'` for local forward lookup.
- Cost: the MCP schema payload gains one more graph tool, so the provider-dependent schema-cache caveat from ADR-018 still applies.

---

## Folded language coverage records

These entries were formerly ADR headings, but they are plain instances of ADR-003's extractor decision rather than standalone decisions under the three-test rule. The task IDs and current behavior stay in ADR-003's per-language table.

| Former entry | Status | Current home |
|--------------|--------|--------------|
| ADR-024 — TypeScript and TSX use dedicated tree-sitter grammars · [T20260505-11] | Superseded by ADR-003 (folded 2026-05) | ADR-003 TypeScript / TSX row |
| ADR-026 — C source and headers share one extractor · [T20260505-16] | Superseded by ADR-003 (folded 2026-05) | ADR-003 C row |
| ADR-027 — Kotlin mirrors Java-style tree-sitter extraction · [T20260505-14] | Superseded by ADR-003 (folded 2026-05) | ADR-003 Kotlin row |
| ADR-028 — C# uses syntax-only enterprise coverage · [T20260505-13] | Superseded by ADR-003 (folded 2026-05) | ADR-003 C# row |

---

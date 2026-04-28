# User Interface — Decisions

**Status:** Draft
**Owner:** gemini
**Last updated:** 2026-04-28

This document records the architectural and design decisions for the Orbit User Interface. It is append-only.

## ADR-001 — Canon Refined Aesthetic

**Status:** Proposed · 2026-04 · [T20260427-29]

**Context.** The dashboard and website require a cohesive visual identity. The previous "Trading Terminal" aesthetic (pure monospace, sharp corners, harsh neon colors) proved too rigid and inaccessible for complex, hierarchical data. We need a design that maintains high density and a "pro-tool" feel while improving readability and structure.

**Decision.** We adopt the "Canon Refined" aesthetic (layered dark mode, dual typography using `Inter` and `JetBrains Mono`, soft semantic colors, and subtle border radii).

**Consequences.**
- We gain high data density and a modern, accessible "pro-tool" visual brand.
- We drop the strict adherence to retro constraints, allowing for standard web affordances.
- Cost: We must formalize a design system to prevent the aesthetic from drifting into generic "Web 2.0" styling, requiring more disciplined CSS architecture.

## ADR-002 — Unified Denial Sources for Policy Dashboard

**Status:** Accepted · 2026-04 · [T20260428-13]

**Context.** The Denials 24h tile counts SQLite audit rows and v2 loop denials, while the Policy tab originally scanned only v2 loop JSONL files. Direct CLI policy denials such as `fs.read` workspace-boundary failures could therefore increment the tile while the detail panel claimed there were no denials.

**Decision.** The dashboard policy-denials endpoint aggregates both v2 denial envelopes and SQLite `status = denied` audit events before building the profile, target, run, and agent tables. SQLite filesystem denials without a real activity fsProfile use the stable `workspace-boundary` profile label.

**Consequences.**
- The Policy tab is now a faithful drill-down for the Denials 24h tile.
- Operators can inspect direct `orbit tool run` policy denials without switching to the raw audit events table.
- Cost: The endpoint carries a small translation layer for SQLite audit rows because that schema does not store typed denial fields like `profile` and `path`.

## ADR-003 — Compact Scoreboard Ratio Columns

**Status:** Accepted · 2026-04 · [T20260428-15]

**Context.** The dashboard scoreboard accumulated separate columns for output tokens, tool calls, duel wins/losses, and friction triage outcomes. After failed tool calls became a first-class summary field, keeping raw counters split across the table made the reliability signal harder to scan and pushed lower-priority triage details into the primary view.

**Decision.** Render companion metrics as compact display pairs in the scoreboard table: `tokens` shows `total/output`, `tool fail/all` shows failed tool calls over total tool calls, and `duel w/all` shows wins over participated duels. Keep only friction reports in the primary table for now, leaving accepted and rejected friction counts to detailed or future drill-down surfaces.

**Consequences.**
- The scoreboard keeps the same backing JSON fields while presenting reliability and participation context in fewer columns.
- `0/N` tool failures remains visually meaningful instead of being dimmed as missing data.
- Cost: Users who want friction accepted/rejected counts or raw duel losses must inspect the underlying summary JSON or a future detail view rather than the primary table.

## Task References

- [T20260427-29]
- [T20260428-13]
- [T20260428-15]

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---
title: <Feature> — Decisions
owner: <agent family: codex | claude | grok | gemini>
last_updated: YYYY-MM-DD
status: Draft
feature: <feature-slug>
doc_role: decisions
type: design
summary: <one-line hook for agent retrieval — non-empty, single line>
tags: [<feature-slug>]
paths: ["crates/<crate>/**"]
related_features: [<feature-slug>]
related_artifacts: [ADR-NNNN]
---

# <Feature> — Decisions

ADR log for <feature>. Entries are append-only and ordered by ascending global
ID. **Allocate the global `ADR-NNNN` via `orbit.adr.add` before writing the
heading** — never hand-author a four-digit number. The store owns ID, status,
owner, and links; this file is the long-form narrative keyed on that same ID.
See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full rules
(when a decision earns an ADR, the mandatory Cost line, rollups).

<!-- Copy the block below for each new ADR. Delete this comment in real docs. -->

## ADR-NNNN — <short title, noun phrase>

**Status:** <Accepted | Proposed | Superseded by ADR-MMMM> · YYYY-MM · [ORB-NNNNN]

**Context.** <1–3 sentences. Why this forced a decision.>

**Decision.** <1–3 sentences. What we chose.>

**Consequences.**

- <consequence>
- Cost: <explicit tradeoff — every ADR must name at least one cost a reader
  could not infer from the decision itself>

## Task References

- [ORB-NNNNN] — <verb phrase: what the task did>

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

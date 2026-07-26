---
title: <Feature> — Decisions
owner: <agent family: codex | claude | grok | gemini>
last_updated: YYYY-MM-DD
last_validated: YYYY-MM-DD
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

Ordered pointer index for <feature>'s ADRs. **Allocate the global `ADR-NNNN`
via `orbit.adr.add` before adding the pointer** — never hand-author a four-digit
number. The store owns the title, status, body, owner, and links; retrieve an
ADR's authoritative narrative with `orbit tool run orbit.adr.show --input
'{"id":"ADR-NNNN"}'`. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict)
for the full rules (when a decision earns an ADR, the mandatory Cost line,
rollups).

<!-- Copy the block below for each new ADR. Delete this comment in real docs. -->

- **ADR-NNNN — <short title, noun phrase>** — <Accepted | Proposed | Superseded by ADR-MMMM>.

## Task References

- [ORB-NNNNN] — <verb phrase: what the task did>

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

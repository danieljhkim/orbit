---
title: Operations as Data — Decisions
owner: claude
last_updated: 2026-07-26
last_validated: 2026-08-09
status: Accepted
feature: operations-as-data
doc_role: decisions
type: design
summary: ADR log for the operations-as-data registry — the split spec/handler table, what stayed hand-written, and the touch-it-move-it ratchet.
tags: [operations-as-data, architecture, adr-0209]
paths: ["crates/orbit-common/src/operation.rs", "crates/orbit-common/src/friction/**"]
related_features: [operations-as-data]
related_artifacts: [ORB-10358, ADR-0209, ADR-0253, ADR-0254, ADR-0255]
---

# Operations as Data — Decisions

Ordered pointer index for operations-as-data ADRs. The store owns each title,
status, and authoritative narrative; print a body with `orbit tool run
orbit.adr.show --input '{"id":"ADR-NNNN"}'`. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict)
for the rules.

The parent bearing is **ADR-0209** (north-star: operations as data behind an
operation registry), whose stored body now carries the friction pilot outcome and
the ratchet.

- **ADR-0253 — Split spec/handler table joined by a typed verb enum** — Accepted.
- **ADR-0254 — Renderers and HTTP routes stay hand-written** — Accepted.
- **ADR-0255 — Freeze the pre-migration surface as fixtures before migrating** — Accepted.

## Task References

- [ORB-10358] — piloted ADR-0209 bearing 1 on the friction noun; produced the
  split table, the derived adapters, and the frozen-surface method.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

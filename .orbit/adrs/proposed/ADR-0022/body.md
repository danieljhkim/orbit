## Context
`orbit.adr.add` could enforce non-empty `related_tasks` at creation, ensuring every ADR has implementation behind it. Alternatively, it can accept empty `related_tasks` and only require task IDs at the `proposed → accepted` transition (per ADR-001's lifecycle rule). The stricter version prevents speculative ADRs that never ship; the looser version respects the natural workflow where decisions get written down before tasks are filed.

## Decision
Empty `related_tasks` at creation is permitted. The task requirement applies only to the `proposed → accepted` transition. Keep the surface simple, see how the corpus behaves in practice, tighten later if proliferation becomes a real problem.

## Consequences

- Design exploration can produce a proposed ADR before its task is filed — a common workflow ("write down the decision while it's fresh, file the task to land it tomorrow").
- Lifecycle still enforces task linkage at the transition that matters (acceptance), so the corpus doesn't accept untied decisions.
- The "iterate before constraining" framing is a deliberate signal: this rule is the most likely to be reconsidered if behavior on the corpus suggests it should.
- Cost: corpus may accumulate proposed-but-never-shipped ADRs that never get cleaned up. No automated GC; reliance on owner discipline (via the lead-responsibility rule in CLAUDE.md). The §1.3 follow-up in [3_vision.md](./3_vision.md) tracks the revisit.

---

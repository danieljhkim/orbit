## Context
Hand-authored relation graphs in issue trackers are universally sparse. The Linear/Jira "related issues" field is empty in 80%+ of records by anecdote; the literature on issue-tracker mining exists precisely because the manual link graph is too sparse to be queryable. Asking Orbit's authoring agents to populate a `related_task_ids` field reliably reproduces the failure mode the prior art suffers from.

## Decision
Edges are *derived first* from three signal sources in the minimal Phase 1 ([2_design.md §2.1](./2_design.md)): `commit-grep`, `kg-attribution`, `task-text`. Three additional derivers (`adr-text`, `runtime-link`, `review-cite`) are deferred to follow-up phases when their consumers ship. Declared edges are accepted only when no derivation source can produce them.

## Consequences
- The graph is dense from day one, before any human or agent does any link curation.
- The deriver pipeline becomes the load-bearing component of lineage's correctness story. Bugs in derivation produce wrong edges system-wide; correctness improvement happens at the deriver, not at the edge.
- Provenance per edge ([2_design.md §1.2](./2_design.md)) means wrong edges are flagged into per-source feedback rather than corrected one row at a time.
- Cost: writing six derivers is more upfront work than letting humans link manually. Confidence calibration becomes a real problem (see [3_vision.md §1.1](./3_vision.md)) that hand-authored systems sidestep by treating every link as 1.0 confidence.

---

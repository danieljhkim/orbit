## Context
Initial discussion of task sync framed it as a v1 feature — small, opt-in, fits the existing per-engineer doctrine. Subsequent analysis (specifically the conflict-resolution scenarios in [2_design.md §3.1](./2_design.md)) revealed that doing sync correctly requires the operation-aware-replay subsystem in ADR-002, which is meaningful engineering. A half-built sync — for example, ADD-only with no update propagation — produces the wrong mental model: "I can see Bob's task exists but never see him work on it." That's worse for adoption than no sync.

## Decision
v1 ships per-engineer with no task sync. The default config is `[task.sync] enabled = false` and the sync code path is absent. This design exists in v1 as a docs artifact only. v2 ships sync as an opt-in feature once the operation-aware-replay subsystem and the structured-conflict UX are real.

## Consequences
- v1 documentation can confidently say "task sync ships in v2" without weasel wording.
- The conflict-resolution work happens in v2 with adequate scope, not in v1 as a rushed addition.
- The decision to defer is itself documented, so v2 reviewers can challenge it on the same grounds future readers could challenge the v2 mechanism.
- Cost: teams who want shared task visibility *now* don't get it from Orbit; they coordinate via existing git/PR workflows or wait for v2.

---

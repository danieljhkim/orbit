## Context
Activities mutate the graph as they edit code. Those mutations must not perturb the persisted store mid-turn (cache stability, concurrent-reader safety). Two implementations are plausible: in-memory overlay vs. per-activity disk staging. Working-graph edit buffering, version chains, and insertion support landed in [T20260409-0656]; write-anchor validation and atomicity guarantees followed in the [T20260416-0236] series (`-2` conflict/audit, `-3` canonical selectors, `-4` atomic moves). Source-file durability on edit ops was added in [T20260417-0302].

## Decision
Keep the working graph in memory for the duration of an activity. Persist at activity boundaries only.

## Consequences
- Branch ref stays byte-stable inside an activity — queries are reproducible.
- Zero disk churn for reads-only activities.
- Cost: a crashed long activity loses its staging. Recovery = rerun. See [3_vision.md §1.3].

---

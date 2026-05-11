## Context
A leaf touched by a reverted task and by a shipped task currently carries both IDs with no distinction. Consumers that want "which change is live" signal have to join against task status externally.

## Decision
Keep `task_ids` a flat union at the graph layer. Do not embed lifecycle state in the graph.

## Consequences
- Graph stays independent of task-state evolution (which changes more often than code structure).
- Cost: consumers wanting shipped-only signal pay the external-join hit every query. [3_vision.md §1.11] may reopen this if the join proves too painful.

---

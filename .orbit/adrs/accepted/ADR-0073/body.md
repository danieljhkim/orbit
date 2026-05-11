## Context
Hashing and extractor dispatch were fully sequential even though each file can be read, hashed, and parsed independently. The graph writer is content-addressed, so any parallel implementation also had to preserve the previous file-order leaf stream and the unchanged-file reuse path from [T20260426-0140].

## Decision
Add `rayon` to `orbit-knowledge` and parallelize only the per-file work. `compute_hashes` runs file reads and SHA-256 computation in workers, then replaces `ctx.new_hashes` after collecting the results. `build_graph_leaves` workers return per-file outputs or reusable prior snapshots; the main thread sorts by original `FileNode` index and mutates `ctx.graph.files` / `ctx.graph.leaves`.

## Consequences
- Full rebuilds can use available cores during the two most expensive file-local stages.
- `PipelineContext` remains single-owner mutable state, so the implementation avoids graph-level locks and shared mutation.
- Deterministic reassembly keeps `ctx.graph.leaves`, `FileNode.leaf_children`, and root object hashes stable relative to the sequential implementation.
- Cost: `orbit-knowledge` now has a direct `rayon` dependency, and future build-stage refactors must preserve ordered collection rather than pushing graph state from worker threads.

---

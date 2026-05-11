## Context
An earlier prototype factored graph-node locks into a standalone `orbit-lock` crate. The crate added a dependency edge without buying reuse — no consumer other than `orbit-knowledge` ever imported it.

## Decision
Keep a file-based shared lock store inside `orbit-knowledge::lock`. Remove the `orbit-lock` crate.

## Consequences
- One fewer crate in the architecture diagram; simpler dependency graph.
- Locks remain file-backed and process-shareable, matching the content-addressed store's on-disk model.
- Cost: if a second consumer ever needs the same lock semantics, we'll re-extract — the shared-crate refactor would have prevented that future churn but would have paid for reuse we don't yet need. [T20260417-0301-2] closed holes around concurrent write paths.

---

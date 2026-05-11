## Context
The graph has to survive crashes mid-rebuild, support concurrent reads during a rebuild, and deduplicate unchanged nodes across builds. A single mutable JSON file fails all three. The original content-addressing refactor landed in [T20260407-0222] (then under `orbit-agent`); the layout stabilized in its current shape during the `orbit-knowledge` consolidation [T20260411-0424].

## Decision
Adopt a git-style split: immutable content-addressed objects under `objects/<hh>/<hash>.json`, immutable blobs under `blobs/<hh>/<hash>.txt`, immutable per-build index under `index/by-id/<root-graph-hash>.json`, and a mutable branch ref as the only pointer that changes.

## Consequences
- Atomic ref swaps via tempfile + rename make interrupted writes safe.
- Object dedup is free because identical content produces identical paths.
- Cost: no GC today — orphan objects accumulate (see [3_vision.md §1.10]).

---

## Context
Selector resolution, name search, and file-symbol counts still walk the hydrated graph or JSON by-id index. A JSON sidecar would keep persistence simple but would not provide efficient prefix/range lookup, partial indexes, or concurrent read/write behavior.

## Decision
Write a mutable SQLite sidecar at `graph/graph_index.sqlite` during `GraphObjectStore::write_graph`. The sidecar is rebuilt in a WAL-backed transaction with `meta`, `node`, and `file_summary` tables; `meta.graph_ref` stores the root graph hash and is inserted last so readers can reject missing or mismatched indexes. Read paths may use the sidecar only after validating that graph ref; `overview` summary uses SQL aggregation for current, unscoped reads and falls back for scoped summaries so prefix semantics stay identical.

## Consequences
- Future read tasks can add SQL fast paths without changing the content-addressed object format.
- Write-path validation can measure the index independently before read behavior depends on it.
- Re-running the same graph preserves semantic meta/node contents, while a new root graph hash cleanly replaces prior rows.
- Schema v3 stores node language because broad overview summaries must return the same language counts as the hydrated fallback.
- Cost: graph persistence now pays an extra SQLite write per rebuild and `orbit-knowledge` directly depends on the workspace `rusqlite` dependency.

---

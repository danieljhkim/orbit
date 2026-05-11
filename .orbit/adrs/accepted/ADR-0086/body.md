## Context
`GraphObjectStore::read_graph` historically hydrated every file and leaf source blob when node objects carried empty `source` plus `source_blob_hash`. Broad tools such as overview, default search, deps, and the history compatibility stub do not need source bodies, so large repositories paid blob I/O on reads whose answers are metadata-only.

## Decision
Add `GraphReadOptions` with separate `hydrate_file_source` and `hydrate_leaf_source` booleans that default to `false`. Tools and services opt in only when they inspect or return source: show hydrates both; refs/callers/implementors hydrate leaves; `source_regex` search hydrates both; pack hydrates leaf bodies only for non-summary output. Incremental rebuild reuse keeps an explicit hydrate-both path because it copies prior source-bearing snapshots.

## Consequences
- Broad metadata reads avoid source blob I/O while preserving the on-disk graph format and all blob hashes.
- Source-returning tools keep their payloads stable by opting in at the load boundary.
- Pack summary mode no longer reads leaf bodies only to discard them.
- Cost: any new reader that touches `leaf.source` or `file.source` must deliberately request hydration; missing that opt-in degrades behavior to empty-source results rather than failing at compile time.

---

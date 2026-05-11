## Context
Default graph search is intentionally substring-capable, but the most common agent searches are exact symbol names and path prefixes. Walking the graph for those shapes burns the same scan cost as broad substring search even though the SQLite sidecar already stores indexed `name_lower` and `location_lower` columns.

## Decision
Route default `orbit.graph.search` queries through `graph_index.sqlite` only when a small classifier identifies an exact-name query or a path-prefix query. Exact-name uses `name_lower = ?`; path-prefix uses `location_lower LIKE ? ESCAPE '\'` after escaping literal `%`, `_`, and `\`. SQL rows are ranked with the same default rank buckets as scan results, and missing/stale indexes, source-regex searches, wildcard/regex-like shapes, and ambiguous substring shapes stay on the scan path.

## Consequences
- Exact-name and path-prefix search can answer from the B-tree sidecar without hydrating the full graph.
- The `node` table carries `scan_order` so SQL ranking can preserve scan-order tie breaks for the same hit set.
- Cost: simple exact probes that miss the name index fall back to scan to preserve substring behavior, so miss latency remains scan-bound until a future substring/trigram index exists.

---

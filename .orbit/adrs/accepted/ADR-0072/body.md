## Context
`KnowledgeStore` selector reads loaded content-addressed graph objects and source blobs through fresh per-call `HashMap`s. Repeated `pack`, `leaf_data`, and history queries against the same store therefore paid disk I/O, JSON parsing, and SHA-256 verification again for immutable hash-addressed data.

## Decision
Add a `GraphObjectCache` owned by `KnowledgeStore`, backed by the `lru` crate with separate object and blob capacities. `read_graph_object` and `extract_leaf_source` consult that shared cache and run hash-integrity verification only on cache miss before insertion.

## Consequences
- Repeated selector reads on the same store avoid redundant object/blob filesystem reads and JSON parsing.
- Cache invalidation is content-hash based: changed nodes naturally use different keys, and old entries age out.
- Store-scoped ownership avoids cross-workspace bleed and keeps tests isolated.
- Cost: separate `KnowledgeStore` instances and separate CLI processes do not share entries; a future long-lived service cache may need a workspace-keyed layer if store instances are not retained.

---

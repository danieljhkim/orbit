## Context
`orbit semantic` mixed embedding-companion lifecycle (`install`, `uninstall`, `stats`, `index`) with user query verbs (`search`, `related`). The phase-1 search engine now owns both lexical and vector ranking, so leaving queries under `semantic` would make users choose an implementation detail before they search.

## Decision
`orbit semantic` is only the lifecycle namespace for the local embedding companion. `orbit search` is the unified query surface; lexical ranking is the default, `--semantic` opts into hybrid BM25 plus cosine for task vectors, and `--related <id>` performs cosine-neighbor lookup for indexed tasks.

## Consequences
- Establishes a precedent that lifecycle namespaces manage local subsystems while query namespaces describe what users are trying to do.
- `orbit semantic search`, `orbit semantic related`, and `orbit semantic reindex` are hard breaks with no shim because there are no known external consumers yet.
- Per-domain search commands stay untouched for phase 1; a later task decides whether they thin-wrap `orbit search`, demote to filters, or retire.
- Vector index coverage remains task-only today; docs, learnings, and ADRs continue to use lexical matching even when `--semantic` is set.
- Cost: historical audit event names `semantic.search` and `semantic.related` become orphaned event types, accepted because no external audit-history consumers exist yet.
## Context
Three retrieval strategies were on the table:

- **Semantic only.** Strong on vocabulary mismatch; weak on literal-identifier queries (function names, error codes, task IDs, file paths). Ignores SQLite's already-shipped FTS5 BM25 capability.
- **Lexical only.** The status quo without this design. Fast, free, well-understood. Cannot find tasks whose vocabulary doesn't match the query.
- **Hybrid: BM25 + cosine, fused via Reciprocal Rank Fusion.** Both retrievers run in parallel; ranks combine without score calibration. Published research consistently shows hybrid beats either alone across information-retrieval benchmarks.

The third option costs one extra SQL query per search and ~30 lines of fusion code. SQLite ships FTS5 with BM25 built in, so the lexical side is essentially free — the implementation is `CREATE VIRTUAL TABLE tasks_fts USING fts5(...)`. Picking semantic-only would be a deliberate choice to fail on literal-identifier queries, which agents query frequently.

A weighted combination (e.g. `0.6 * cosine_score + 0.4 * bm25_score`) was considered as an alternative fusion. Rejected because BM25 and cosine produce scores on incommensurable scales, weights become a tuning knob with no obvious right answer, and RRF demonstrates equal or better quality without the calibration burden.

## Decision
Phase 1 ships hybrid retrieval. Both retrievers run on every `search` query. RRF (k=60) fuses the rankings. Score breakdown (`bm25_rank`, `cosine_rank`) is exposed in result payloads so consumers can detect which retriever drove a given hit. `related` (similar-task discovery) is cosine-only because lexical similarity adds noise for that use case.

## Consequences
- Literal-identifier queries (task IDs, function names, file paths) match correctly.
- Vocabulary-mismatch queries match correctly.
- Score breakdown gives agents a real signal for confidence calibration without exposing raw incommensurable scores.
- Cost: every `search` runs two SQL queries instead of one and computes one extra fusion pass. At phase-1 latency budgets (target <200ms p95) this is unproblematic, but it doubles the per-query work versus a single-retriever design and that overhead is paid even on queries where one retriever would have been enough.

---

## Context
Orbit already has a global SQLite database at `~/.orbit/orbit.db` for command audit, tool registry, and task-lock bookkeeping. Task bundles themselves are workspace-scoped under `.orbit/tasks`, and the scoping rules treat task data as workspace-only. Semantic rows are derived from task text, so putting embeddings in the global DB would create cross-project leakage and make stale-row accounting depend on which workspace happened to be active.

## Decision
Store phase-1 semantic tables in a workspace-local SQLite database at `.orbit/state/semantic.db`. The semantic feature crate (`orbit-embed`, see ADR-007) opens and owns this file end-to-end: `VectorStore::open(path)` and `VectorStore::open_in_memory()` apply the WAL + busy_timeout pragmas, run `ensure_vector_schema(conn)` (CREATE TABLE IF NOT EXISTS for `embeddings` + `tasks_fts`), and return a `VectorStore` whose `Arc<Mutex<Connection>>` is the only handle into the database.

## Consequences
- Task-derived vectors and FTS rows follow task scoping: one workspace cannot see another workspace's semantic index.
- `orbit semantic reindex` can rebuild only the active workspace without filtering a global table by workspace ID.
- Tests use `VectorStore::open_in_memory()` directly — no orbit-store handle to plumb through.
- `semantic.db` carries only the embeddings/FTS5 schema. Earlier phase-1 implementations co-located the generic `orbit-store` migration bundle in the same file (audit, tools, reservations, etc.); that collateral was removed when ADR-007 cut the `orbit-embed → orbit-store` dependency.

---

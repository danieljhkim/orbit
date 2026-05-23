## Context
[T20260510-9] landed phase-1 with logic split across the wrong crate boundary:

- `crates/orbit-store/src/vector/mod.rs` (757 lines) hosted `VectorStore`, `EmbedWorker`, the BLAKE3 dedup, the paragraph chunker, and the cosine helper. It imported `orbit_embed::{Embedder, SubprocessEmbedder}` directly, so the dep arrow was `orbit-store → orbit-embed` and `orbit-store` leaked knowledge of the embedding feature.
- `crates/orbit-core/src/command/semantic.rs` (328 lines) hosted install / uninstall / reindex / stats logic plus HTTP companion download, model-dir wrangling, and companion version probe. None of that is `OrbitRuntime` orchestration.

Compare to the analogous knowledge-graph crate: `orbit-knowledge` owns its data + commands in `commands/{search,show,overview,…}.rs` and `orbit-core/src/command/graph.rs` is a six-line re-export. Phase-1 semantic search was three-crate-spread; the graph feature is one-crate-plus-thin-re-export.

## Decision
Relocate semantic-search ownership into `orbit-embed` and make it self-contained:

- Move the vector storage module to `crates/orbit-embed/src/vector/mod.rs`. `orbit-store` drops its dependency on `orbit-embed`. **`orbit-embed` does not depend on `orbit-store`**: it owns its own SQLite handle directly via `rusqlite::Connection` wrapped in `Arc<Mutex<_>>`, applies WAL + busy_timeout pragmas, and runs `ensure_vector_schema` on `VectorStore::open(path)` / `VectorStore::open_in_memory()`. This mirrors how `orbit-knowledge` owns `graph_index.sqlite` end-to-end without going through `orbit-store`.
- Move the per-command logic to `crates/orbit-embed/src/commands/{install,uninstall,reindex,stats}.rs`. Each command file owns its `*Params` and `*Result` types and one public `run` function. `crates/orbit-embed/src/commands/mod.rs` aggregates the surface and holds shared helpers (`parse_model`, `active_model`, `remove_file_if_exists`, `DEFAULT_RELEASE_BASE_URL`).
- Reduce `crates/orbit-core/src/command/semantic.rs` to a thin `OrbitRuntime` delegate (≤45 lines) that re-exports the param/result types and forwards each method to `orbit_embed::commands::*::run`. CLI ergonomics are preserved: `orbit-cli` still calls `runtime.semantic_install(params)` etc., and `Execute` impls are unchanged.

## Consequences
- Crate dependency direction matches the graph feature exactly: `orbit-embed` is a near-leaf feature crate that depends only on `orbit-common` (and its workspace-standard libs: rusqlite, blake3, chrono, reqwest). `orbit-store` is storage-only; both crates are independent.
- `semantic.db` carries only the `embeddings` and `tasks_fts` tables — no audit/tools/reservations/task_tags collateral. Pre-T-20 implementations had `orbit-store::Store::open` apply the full migration bundle to `semantic.db` as a side effect; that's gone now.
- `orbit-embed` gains `reqwest` (blocking) for the install download and inlines a small WAL helper. Neither violates ADR-005's slim-client constraint: the prohibition is on linking ML inference (fastembed-rs / ONNX Runtime) into the main `orbit` binary; storage and HTTP client are fine. `orbit-embed-companion` remains the only crate that links fastembed.
- The phase-2 graph corpus (per ADR-002, ADR-003) can land in `orbit-embed::vector` directly without crossing another crate boundary.
- The phase-1 CLI surface is preserved exactly (install / uninstall / reindex / stats produce identical observable output). `VectorStore::new(store)` is replaced by `VectorStore::open(path)` and `VectorStore::open_in_memory()`. Only one in-tree call site (`crates/orbit-core/src/runtime/builder.rs`) is affected: it stops opening `Store::open(&persistence.semantic_db)` and instead calls `VectorStore::open(&persistence.semantic_db)` directly.
- Cost: a small amount of `rusqlite::Connection` plumbing duplicates what `orbit-store::Store` does (WAL pragma helper, parent-dir creation, mutex wrapping). The duplication is small (≈30 lines) and isolates the semantic feature's schema from migrations to other store domains, which is the whole point.

---

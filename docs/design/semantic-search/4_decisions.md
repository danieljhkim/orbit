# Semantic Search — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-10

ADR-style log of non-obvious semantic-search decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. Every ADR names at least one cost. ADRs in this file carry status `Proposed` until the implementing task ships; they flip to `Accepted` with the implementing task ID at that point.

---

## ADR-001 — fastembed-rs ONNX backend over Candle, llama.cpp, or external ollama

**Status:** Proposed · 2026-05 · [T20260510-3]

**Context.** Local embedding inference has four plausible backends:

| Backend | Profile |
|---------|---------|
| **fastembed-rs** | Pure Rust crate wrapping ONNX Runtime; ships a small set of well-known sentence-embedding models (BGE, MiniLM, Nomic, mxbai); CPU-only fine; batch-friendly. |
| **Candle** | Pure-Rust ML framework from HuggingFace; broader model support; more code to integrate; less plug-and-play for embeddings specifically. |
| **llama-cpp-rs** | Bindings to llama.cpp; GGUF format; runs anything from tiny embedding models to large LLMs; optional GPU; C++ build dependency. |
| **External ollama or similar always-on daemon** | Outsources inference but requires the user to install and run a separate long-lived process. |

This ADR addresses *which* backend to use. The orthogonal decision of *how* the backend is delivered to the user (in-process vs. companion binary vs. feature flag) is in [ADR-005](#adr-005--companion-binary-installed-on-demand-rather-than-bundled-in-orbit). Within in-process or in-companion options, fastembed-rs covers the embedding-model use case directly; Candle is more general but requires more Orbit-side code; llama-cpp-rs is overkill and adds a C++ build dependency that complicates Orbit's release pipeline. An always-on ollama-style daemon contradicts Orbit's no-daemon posture regardless of binary placement.

**Decision.** Phase 1 uses fastembed-rs as the inference backend, exposed through an `Embedder` trait that lives in a new `orbit-embed` library crate. Per ADR-005, fastembed-rs is linked into a separate `orbit-embed-companion` binary, not into the main `orbit` binary; the trait abstraction means an alternative backend can later swap in without touching `orbit-store` or `orbit-tools`. The user-facing default model is BGE-small-en-v1.5 (384 dim, ~30MB), with `--model {bge-small | minilm-l6 | nomic-v1.5}` selected at install time. Reject external always-on ollama: contradicts the no-daemon posture. Reject llama-cpp-rs: C++ build dependency outweighs its flexibility for embedding-only work. Reject Candle as default: more integration work for less out-of-the-box behavior; remains a viable trait-impl swap.

**Consequences.**
- The `Embedder` trait isolates the choice of backend from storage and retrieval; later-arriving backends (Candle, code-tuned models) plug in without schema or query changes.
- The fastembed-rs model catalog (BGE, MiniLM, Nomic, mxbai) is the menu phase-1 users pick from. Other model families require a new `Embedder` impl, not a config change.
- Model output is well-characterized by published benchmarks (MTEB) so the default is defensible without an Orbit-specific eval ([3_vision.md §1.1](./3_vision.md)).
- Cost: locking in to the fastembed-rs catalog means models outside that catalog (e.g., voyage-code, code-tuned models in [3_vision.md §1.7](./3_vision.md)) need a different `Embedder` impl in a future task. The trait abstraction makes that mechanical, but it does mean the phase-1 menu is bounded by what fastembed-rs ships.

---

## ADR-002 — Brute-force cosine over SQLite BLOBs; `sqlite-vec` reserved as phase-2 upgrade

**Status:** Proposed · 2026-05 · [T20260510-3]

**Context.** Vector storage and retrieval has three plausible shapes:

1. **Brute-force cosine in Rust over SQLite BLOBs.** No new dependency. Linear scan per query.
2. **`sqlite-vec` loadable extension.** HNSW-indexed nearest-neighbor inside SQLite. Same on-disk format as (1). Adds a runtime extension load.
3. **Standalone vector DB** (Qdrant, LanceDB, ChromaDB). Production-grade. Adds a binary dependency or sidecar.

At phase-1 scale — tasks-only, low thousands of artifacts × small number of fields per task = tens of thousands of vectors at 384d — brute-force cosine is sub-100ms on a modern laptop and zero new dependencies. `sqlite-vec` is the right answer once the corpus crosses ~100K vectors; that crossing happens with phase-2 graph integration, not phase 1. Standalone vector DBs are inappropriate for an embedded local tool.

A subtle point: the choice of `embedding BLOB` storage format in (1) is forward-compatible with `sqlite-vec`. Upgrading is a CREATE VIRTUAL TABLE plus an INSERT … SELECT, not a schema rewrite.

**Decision.** Phase 1 implements brute-force cosine in Rust over `embeddings.embedding` BLOBs. The schema preserves forward compatibility with `sqlite-vec` (same BLOB layout, same `dim` and `model_id` columns). Phase 2's graph corpus revisits storage as a separate ADR; if `sqlite-vec` is the right call at that point, it's an additive change, not a migration.

**Consequences.**
- Zero new runtime dependencies in phase 1.
- Schema and on-disk layout are stable across the phase-1/phase-2 boundary.
- Query latency is acceptable until the corpus crosses ~100K vectors.
- Cost: brute force scans every row every query. For a stable phase-1 corpus that's fine, but it means we can't ship "semantic search across the entire repository graph" without revisiting storage. The decision deliberately scopes phase 1 to where brute force is comfortable, and pays the upgrade cost later when there is operational evidence to size against.

---

## ADR-003 — Per-field embeddings with chunked overflow, not whole-bundle concatenation

**Status:** Proposed · 2026-05 · [T20260510-3]

**Context.** A task bundle has structurally distinct fields (purpose, summary, plan, acceptance criteria, comments, review threads) of widely varying length. Two embedding strategies exist:

- **Concatenate everything into one document and embed once.** Simplest; one row per task. Loses precision because a strong match in `purpose` is averaged with weak signal from twenty unrelated comments. Long bundles routinely exceed BGE-small's 512-token context, forcing arbitrary truncation.
- **Per-field embeddings, with long fields chunked at paragraph boundaries.** Multiple rows per task. Best-matching field surfaces in the result. Chunking handles the context-window limit cleanly.

The cost of per-field is mostly storage (~5–20× rows per task) and indexing CPU. At BGE-small's 384d, even a generous 20 rows × 10K tasks = 200K rows × 1.5KB = 300MB. Fits comfortably in SQLite, comfortable for brute force at this scale.

**Decision.** Phase 1 indexes one row per `(task_id, field, chunk_idx)`. Result formatting collapses multiple field hits on the same task to a single result with the highest-scoring field surfaced as the snippet. Long fields (`plan.md`, `execution-summary.md`) are split at paragraph boundaries with a target of 400 tokens per chunk and 50-token overlap.

**Consequences.**
- Result snippets point to the actual field that matched, which makes the answer interpretable to users and agents.
- Comments and review messages become independently findable, which directly addresses the "decisions buried in long threads" failure mode in [1_overview.md §1](./1_overview.md).
- Schema's `field` column carries the discriminator without a separate table.
- Cost: 5–20× more rows per task, more storage, more indexing CPU. At phase-1 scale the cost is unproblematic; at much larger scales the per-field strategy may need revisiting alongside the storage upgrade in ADR-002.

---

## ADR-004 — Hybrid retrieval (FTS5 BM25 + cosine, fused via RRF) from day one

**Status:** Proposed · 2026-05 · [T20260510-3]

**Context.** Three retrieval strategies were on the table:

- **Semantic only.** Strong on vocabulary mismatch; weak on literal-identifier queries (function names, error codes, task IDs, file paths). Ignores SQLite's already-shipped FTS5 BM25 capability.
- **Lexical only.** The status quo without this design. Fast, free, well-understood. Cannot find tasks whose vocabulary doesn't match the query.
- **Hybrid: BM25 + cosine, fused via Reciprocal Rank Fusion.** Both retrievers run in parallel; ranks combine without score calibration. Published research consistently shows hybrid beats either alone across information-retrieval benchmarks.

The third option costs one extra SQL query per search and ~30 lines of fusion code. SQLite ships FTS5 with BM25 built in, so the lexical side is essentially free — the implementation is `CREATE VIRTUAL TABLE tasks_fts USING fts5(...)`. Picking semantic-only would be a deliberate choice to fail on literal-identifier queries, which agents query frequently.

A weighted combination (e.g. `0.6 * cosine_score + 0.4 * bm25_score`) was considered as an alternative fusion. Rejected because BM25 and cosine produce scores on incommensurable scales, weights become a tuning knob with no obvious right answer, and RRF demonstrates equal or better quality without the calibration burden.

**Decision.** Phase 1 ships hybrid retrieval. Both retrievers run on every `search` query. RRF (k=60) fuses the rankings. Score breakdown (`bm25_rank`, `cosine_rank`) is exposed in result payloads so consumers can detect which retriever drove a given hit. `related` (similar-task discovery) is cosine-only because lexical similarity adds noise for that use case.

**Consequences.**
- Literal-identifier queries (task IDs, function names, file paths) match correctly.
- Vocabulary-mismatch queries match correctly.
- Score breakdown gives agents a real signal for confidence calibration without exposing raw incommensurable scores.
- Cost: every `search` runs two SQL queries instead of one and computes one extra fusion pass. At phase-1 latency budgets (target <200ms p95) this is unproblematic, but it doubles the per-query work versus a single-retriever design and that overhead is paid even on queries where one retriever would have been enough.

---

## ADR-005 — Companion binary installed on demand, rather than bundled in `orbit`

**Status:** Proposed · 2026-05 · [T20260510-3]

**Context.** Once fastembed-rs is the chosen backend (ADR-001), the question of where it lives matters. Linking ONNX Runtime + fastembed-rs into the main `orbit` binary adds ~50MB and pays that cost for every user — including users who never invoke semantic search. Three packaging shapes are plausible:

| Option | Default install size | Opt-in mechanism | Inference latency |
|--------|----------------------|------------------|-------------------|
| **A. Bundled in `orbit`** | Large (~50MB+) | None (always available) | In-process; instant after warm cache |
| **B. Cargo feature flag, two release artifacts** | Small or large depending on which artifact you download | Choose `orbit-full` at install time; replace the binary to swap | In-process; instant |
| **C. Companion binary downloaded on demand** | Small | `orbit semantic install [--model X]` | Subprocess; ~100–300ms ORT cold start, amortized across batches |

Option A is what the design originally called "single binary install posture preserved." It does preserve that, but it also means the always-pay binary cost is a permanent tax on users who don't want semantic search. Option B requires users to swap their main binary, which is gross UX (in-flight processes, partially-applied upgrades, surprising behavior changes). Option C keeps the default install slim and gives the user explicit control over which model — and how much disk — they're committing to, at the cost of subprocess overhead.

**Decision.** Phase 1 ships option C. Two new crates:

- `orbit-embed` — small library holding the `Embedder` trait, JSON-RPC types, and `SubprocessEmbedder` (the trait impl that locates and talks to the companion). No fastembed-rs dependency. Linked into the main `orbit` binary.
- `orbit-embed-companion` — binary crate. Depends on `orbit-embed` + fastembed-rs. Produces a standalone `orbit-embed-companion` binary distributed via GitHub Releases per platform.

`orbit semantic install [--model bge-small | minilm-l6 | nomic-v1.5]` downloads the platform-appropriate companion binary plus the chosen model files into `~/.orbit/embed/`. Inference happens via stdio JSON-RPC; the subprocess is kept alive across a batch (`reindex`, multi-query session) and shut down at process exit. `orbit semantic uninstall` removes both the companion and the model. When semantic search is invoked without the companion installed, all read/write paths fail with a clear, actionable error pointing at `orbit semantic install`.

**Consequences.**
- Default `orbit` install stays slim — no ORT, no fastembed-rs in the main binary. Users who don't want semantic search pay no cost.
- The model menu is exposed at install time, not as a runtime config knob the user has to discover. Users actively choose between MiniLM-L6 (smallest, ~23MB), BGE-small (default, ~30MB), and Nomic-v1.5 (largest, ~140MB) at the moment they're committing to the feature.
- The subprocess-RPC boundary makes the companion swappable: a future `orbit-embed-companion-candle` could reuse the same RPC protocol with a different inference engine.
- Cost: install becomes a two-step user action (`orbit` install, then `orbit semantic install`). Users hitting `orbit semantic search` without the companion installed need a clean, helpful error. The subprocess introduces ~100–300ms ORT cold-start latency per process; mitigated by reusing the subprocess across batches but still visible on first interactive query. Additionally, the companion binary requires a per-platform release pipeline (Linux x86_64, Linux arm64, macOS x86_64, macOS arm64, Windows x86_64), which is real release-engineering work for follow-up tasks.

---

## Task References

- [T20260510-3] — Design semantic search over task artifacts and graph (v2). The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

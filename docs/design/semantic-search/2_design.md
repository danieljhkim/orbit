# Semantic Search — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-10

This document specifies phase-1 semantic search: the two new `orbit-embed*` crates and their place in the architecture, the companion-binary inference model, the SQLite vector storage schema, the per-field embedding strategy, the hybrid (BM25 + cosine) retrieval pipeline, the MCP and CLI surface, the index-maintenance lifecycle, and the concerns the design deliberately leaves to follow-ups.

---

## 1. Architectural Placement

Two new crates land:

- **`orbit-embed`** — small client library. Owns the `Embedder` trait, the JSON-RPC request/response types, and `SubprocessEmbedder` (the trait impl that locates and talks to the companion). Depends only on `orbit-common`. **Does not depend on fastembed-rs**, so it adds negligible binary cost. Linked into the main `orbit` binary.
- **`orbit-embed-companion`** — binary crate. Depends on `orbit-embed` (for the RPC types) and on fastembed-rs (for the actual ONNX inference). Produces the standalone `orbit-embed-companion` executable distributed via GitHub Releases per platform. **Not built into `orbit`**; users opt in by running `orbit semantic install`. Per [4_decisions.md ADR-005](./4_decisions.md).

Updated dependency graph:

```
orbit-common → orbit-embed → orbit-store → ... (existing graph; orbit-embed has no fastembed dep)
                          ↘ orbit-embed-companion (separate crate, separate binary, fastembed-rs lives here)
```

`orbit-store` gains a new submodule `vector::` alongside the existing `file::` and `sqlite::` layers. `vector::` owns the `embeddings` table schema, write/upsert/delete API, and the brute-force cosine query implementation. It depends on `orbit-embed` for the `Embedder` trait but treats the embedder as injected — tests pass a `NoopEmbedder` that returns deterministic vectors so unit tests never need the companion to be installed.

`orbit-tools` exposes `orbit.semantic.search` and `orbit.semantic.related` as MCP tools, and `orbit-cli` exposes `orbit semantic` subcommands (including `install` and `uninstall`). Both are thin shells over `orbit-store::vector::*`.

---

## 2. Inference Backend

### 2.1 Trait

Defined in `orbit-embed`:

```rust
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;        // e.g. "bge-small-en-v1.5"
    fn dim(&self) -> usize;            // e.g. 384
    fn max_input_tokens(&self) -> usize; // e.g. 512
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError>;
    fn token_count(&self, text: &str) -> Result<usize, OrbitError>;
}
```

Batch input is mandatory — fastembed-rs is meaningfully faster on batches than on single-document calls because of ONNX kernel reuse, and the indexing path naturally batches by task. `token_count` is exposed because the chunker in [§4.2](#42-chunking-long-fields) needs exact token counts to split fields under the model's context limit.

### 2.2 Companion-binary architecture

Phase 1 ships a single trait impl: `SubprocessEmbedder` (in `orbit-embed`). It does not perform inference itself — it spawns and talks to the `orbit-embed-companion` binary that the user installed with `orbit semantic install`. The arrangement looks like:

```
orbit (main binary)                          orbit-embed-companion (installed binary)
├── SubprocessEmbedder                       ├── fastembed-rs
│     ↕ stdio JSON-RPC                       │     ↕ ONNX Runtime
└── orbit-store::vector                      └── BGE-small / MiniLM-L6 / Nomic-v1.5
```

Lifecycle:

- `SubprocessEmbedder::new()` resolves the companion path under `~/.orbit/embed/bin/orbit-embed-companion-<platform>` and starts the subprocess. ~100–300ms cold-start latency for ORT init.
- The subprocess stays alive for the duration of the parent process or until explicitly dropped. Indexing batches and multi-query interactive sessions reuse the same subprocess.
- On process exit, the parent sends an `exit` RPC and waits up to 1s; if unresponsive, sends SIGTERM.

### 2.3 RPC protocol

JSON Lines over stdio. Each request and response is a single JSON object on a single line.

```jsonc
// Request
{"id": 1, "method": "info"}
{"id": 2, "method": "embed", "texts": ["hello", "world"]}
{"id": 3, "method": "token_count", "text": "..."}
{"id": 4, "method": "exit"}

// Response
{"id": 1, "result": {"model_id": "bge-small-en-v1.5", "dim": 384, "max_input_tokens": 512}}
{"id": 2, "result": {"vectors": [[...384 floats...], [...384 floats...]]}}
{"id": 3, "result": {"tokens": 42}}
{"id": 4, "result": {"ok": true}}

// Error
{"id": 2, "error": {"code": "model_load_failed", "message": "..."}}
```

The protocol is intentionally minimal — four methods, no streaming, no auth. The trust boundary is "this is a binary the user installed under their home directory"; there is no network involvement and no multi-tenant concern.

### 2.4 Default model and install-time model selection

`orbit semantic install` accepts `--model {bge-small | minilm-l6 | nomic-v1.5}`; default is `bge-small`. Model files are downloaded into `~/.orbit/embed/models/<model_id>/`. Switching model means running `orbit semantic install --model OTHER`, which downloads the new model alongside the existing one (so the embeddings under the old `model_id` keep working until reindexed; see [§7.2](#72-backfill-and-migration)).

The three supported models per [3_vision.md §1](./3_vision.md):

| Model | Dim | On-disk | Best for |
|-------|-----|---------|----------|
| `minilm-l6` (all-MiniLM-L6-v2) | 384 | ~23MB | Smallest disk and fastest CPU; older but battle-tested. |
| `bge-small` (BGE-small-en-v1.5) — default | 384 | ~30MB | Strong recall-per-byte for English on MTEB. |
| `nomic-v1.5` (nomic-embed-text-v1.5) | 768 | ~140MB | Best quality; Matryoshka-truncatable; 8192-token context. |

### 2.5 Companion locator and missing-companion behavior

On first use of any embedder-touching path, `SubprocessEmbedder::new()` checks:

1. `$ORBIT_EMBED_COMPANION` env var → explicit path override (used by tests and airgapped operators).
2. `~/.orbit/embed/bin/orbit-embed-companion-<platform>` → standard install path.
3. `$PATH` → fallback for unusual deployments.

If none resolve, the embedder returns `OrbitError::CompanionNotInstalled` with a remediation message: `"Run \`orbit semantic install\` to enable semantic search."` Indexing-path callers log and skip (semantic search is not on the critical path of task mutation; see [§7.1](#71-on-mutation-indexing)). Query-path callers surface the error directly to the user.

### 2.6 Alternative backends

The trait + RPC protocol make alternative companions viable without changing storage or retrieval. A future `orbit-embed-companion-candle` could speak the same protocol and ship as a separate downloadable. None ship in phase 1; the protocol exists to keep that door open. The full backend comparison is in [4_decisions.md ADR-001](./4_decisions.md); the packaging decision is in [4_decisions.md ADR-005](./4_decisions.md).

---

## 3. Vector Storage

### 3.1 Schema

A new SQLite table in the existing per-workspace store:

```sql
CREATE TABLE embeddings (
    source_kind TEXT NOT NULL,         -- "task" (phase 1); "symbol" (phase 2 reserved)
    source_id   TEXT NOT NULL,         -- task ID or future symbol ID
    field       TEXT NOT NULL,         -- "purpose", "plan", "comment_3", "review_1_msg_2", ...
    chunk_idx   INTEGER NOT NULL,      -- 0 for unchunked; >0 for splits of long fields
    content_hash TEXT NOT NULL,        -- BLAKE3 of the embedded text; cheap re-index gate
    model_id    TEXT NOT NULL,         -- "bge-small-en-v1.5"
    dim         INTEGER NOT NULL,      -- 384
    embedding   BLOB NOT NULL,         -- dim * 4 bytes, native-endian f32
    created_at  TEXT NOT NULL,
    PRIMARY KEY (source_kind, source_id, field, chunk_idx, model_id)
);

CREATE INDEX embeddings_by_source ON embeddings(source_kind, source_id);
CREATE INDEX embeddings_by_model  ON embeddings(model_id);
```

The composite primary key includes `model_id` so embeddings under multiple models can coexist during a model migration. The `content_hash` lets the indexer skip work when the underlying text hasn't changed since the last embed.

### 3.2 Query path (brute force)

```text
1. embed query under default model_id  → query vector q (dim 384)
2. SELECT embedding, source_kind, source_id, field, chunk_idx
     FROM embeddings WHERE model_id = ?
3. for each row: compute cosine(q, row.embedding); maintain a fixed-size top-k heap
4. return top-k (source_id, field, score)
```

At 30k vectors × 384d, the full scan is ~50ms in pure Rust on a modern laptop and dominated by the SQLite read, not the dot product. The implementation uses `f32` slabs and SIMD-friendly contiguous buffers; it does not need an HNSW index at phase-1 scale. The forward path to HNSW (via `sqlite-vec`) is preserved by the schema — `embedding BLOB` is the same shape `sqlite-vec` expects ([4_decisions.md ADR-002](./4_decisions.md)).

### 3.3 Write path

A single `upsert_embeddings` API takes `(source_kind, source_id, fields: Vec<(field, text)>)`. For each field:

1. Compute `content_hash = BLAKE3(text)`.
2. If a row already exists with the same `(source_kind, source_id, field, chunk_idx, model_id)` and matching `content_hash`, skip.
3. Otherwise embed and upsert.

This makes "reindex everything" idempotent and cheap when nothing has changed. Re-embedding only happens on real text changes or model changes.

---

## 4. What to Embed for Tasks

### 4.1 Per-field rather than whole-bundle

A task bundle has structured fields with different retrieval value. The design indexes them as separate rows rather than concatenating into a single document:

| Field | Source | Rationale |
|-------|--------|-----------|
| `purpose` | `task.yaml.purpose` | High-density signal; what the task is for |
| `summary` | `task.yaml.summary` | One-line gist; useful for short-query matches |
| `plan` | `plan.md` | Implementation intent; long-form |
| `execution_summary` | `execution-summary.md` | What actually shipped |
| `acceptance_criteria` | `task.yaml.acceptance_criteria[*]` joined | Often the most query-relevant text |
| `comment_<idx>` | `task.yaml.comments[idx].body` | One row per comment; preserves authorship |
| `review_<thread>_msg_<idx>` | review_threads | Decision context lives here |

A single match in a comment surfaces the parent task; the result formatter rolls field-level hits up to task-level results, with the highest-scoring field shown as a snippet ([4_decisions.md ADR-003](./4_decisions.md)).

### 4.2 Chunking long fields

`plan.md` and `execution-summary.md` regularly exceed BGE's 512-token context. The chunker splits on paragraph boundaries with a target of 400 tokens per chunk and a 50-token overlap. Each chunk gets its own row with `chunk_idx = 0, 1, 2, ...`. Queries that match multiple chunks of the same field/task collapse to one result with the best-scoring chunk surfaced.

Token counting uses fastembed-rs's tokenizer for the active model — exact, not heuristic — to keep chunks below the model's actual limit.

### 4.3 Fields *not* embedded

- `task.yaml.id`, `created_at`, `updated_at`, `status`, `dependencies`, `external_refs`: identifiers and structured metadata; FTS5 handles these better.
- `artifacts/**` blobs: out of scope phase 1. Most are binary or large generated content; embedding them is expensive and rarely useful for "find the related task" queries.

---

## 5. Hybrid Retrieval

### 5.1 FTS5 virtual table

A second new table mirrors task content for lexical search:

```sql
CREATE VIRTUAL TABLE tasks_fts USING fts5(
    source_id UNINDEXED,
    field UNINDEXED,
    content,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
```

Populated from the same per-field text as the embedding indexer. FTS5 ships with BM25 ranking built in — no implementation needed beyond the virtual table.

### 5.2 Reciprocal Rank Fusion

Both retrievers run in parallel for a query. Each returns a ranked list of `(source_id, field, chunk_idx)` candidates. RRF combines them:

```
score(c) = Σ over retrievers r of: 1 / (k + rank_r(c))
```

with `k = 60` (the published-paper default that has held up across many evaluations). The fused ranking determines the final result order. RRF is parameter-light, requires no score calibration between retrievers, and consistently beats either retriever alone in the literature.

### 5.3 Why hybrid

Three queries that motivate the choice:

- **"slow embed inference"** — semantic wins; lexical misses tasks titled "BGE latency degraded after Nomic swap."
- **"T20260421-0528"** — lexical wins; semantic returns near-random because the literal token has no semantic neighborhood.
- **"file: orbit-store/src/file/task_store/layout.rs"** — lexical wins; literal path tokens dominate.

Either retriever alone has a failure mode the other doesn't. RRF resolves both at the cost of one extra SQL query per search ([4_decisions.md ADR-004](./4_decisions.md)).

---

## 6. CLI and MCP Surface

### 6.1 CLI

```
orbit semantic install   [--model bge-small | minilm-l6 | nomic-v1.5]
orbit semantic uninstall [--model MODEL] [--all]
orbit semantic search    <query> [--limit N] [--field FIELD] [--kind task]
orbit semantic related   <task-id> [--limit N]
orbit semantic reindex   [--force] [--model MODEL]
orbit semantic stats
```

`install` is the gate that enables every other subcommand. It downloads the platform-appropriate `orbit-embed-companion` binary from the published release URL and the chosen model from HuggingFace, both into `~/.orbit/embed/`. Default model is `bge-small`; users can override per [§2.4](#24-default-model-and-install-time-model-selection). Re-running `install` with a different `--model` adds that model alongside existing ones.

`uninstall` removes the companion binary and (by default) the currently active model. `--model M` removes only model M. `--all` removes the companion plus every installed model.

`search` runs the hybrid pipeline. `related` embeds the target task's `purpose + summary` and runs cosine-only against other tasks (lexical fusion adds noise here). `reindex` rebuilds the `embeddings` rows; `--force` ignores `content_hash` and re-embeds everything. `stats` reports row counts, model distribution, stale-row count, and companion-install status.

If the companion is not installed, `search`, `related`, and `reindex` exit non-zero with: `"Semantic search not enabled. Run \`orbit semantic install\` to download the inference companion."`

### 6.2 MCP tools

- `orbit.semantic.search` — `(query, limit?, kind?, field?)` → ranked results with snippets.
- `orbit.semantic.related` — `(task_id, limit?)` → ranked similar tasks.

Both tools are read-only. Indexing is implicit (on task mutation) or explicit (`orbit semantic reindex`); no MCP write surface.

### 6.3 Result shape

```jsonc
{
  "results": [
    {
      "source_kind": "task",
      "source_id": "T20260421-0528",
      "best_field": "plan",
      "snippet": "...",
      "score": 0.87,
      "score_breakdown": { "rrf": 0.87, "bm25_rank": 4, "cosine_rank": 1 }
    }
  ],
  "model_id": "bge-small-en-v1.5"
}
```

The score breakdown is deliberately exposed: agents can use it to decide whether a hit is "lexical exact match" vs. "semantic neighborhood" and adapt downstream behavior.

---

## 7. Index Maintenance

### 7.1 On-mutation indexing

`task.add` and mutating `task.update` paths emit an `EmbedJob` to a bounded in-process channel after the durable write commits. A worker drains the channel, batches up to 16 jobs at a time, and runs `upsert_embeddings`. Failures log and continue — embedding is not in the critical path of task mutation. Users without the companion installed (`orbit semantic install` not yet run) see `OrbitError::CompanionNotInstalled` from the worker, which it logs at debug level and skips; core task operations are entirely unaffected.

### 7.2 Backfill and migration

`orbit semantic reindex` walks the task store and embeds anything not present (or whose `content_hash` differs). A model migration (`--model`) writes new rows under the new `model_id`; the old `model_id` rows can be deleted in a follow-up `orbit semantic prune --model OLD`.

### 7.3 Deletion

`task.delete` cascades to `DELETE FROM embeddings WHERE source_kind = 'task' AND source_id = ?`. Tombstoned tasks (in the v2 task-sync sense, see [docs/design/task-sync/](../task-sync/)) are not embedded.

---

## 8. Concerns & Honest Limitations

### 8.1 Two-step install and first-run download

Users who want semantic search must run two commands instead of one: install `orbit`, then run `orbit semantic install [--model X]` to download the companion (~50MB) and the chosen model (~23–140MB). The install command is the friction; the per-model download afterward is the same content cost a bundled design would have charged on first search. For users behind corporate proxies or in airgapped environments the friction multiplies — see [3_vision.md §1.2](./3_vision.md). The cost is explicit in [4_decisions.md ADR-005](./4_decisions.md); the mitigation is a clear `CompanionNotInstalled` error with the exact remediation command.

### 8.2 Subprocess overhead

The companion lives in a separate process and inference happens via stdio JSON-RPC. Cold-start latency is ~100–300ms (ORT init + model load). The subprocess is reused across a batch (`reindex`) and across a multi-query interactive session, so the cost is amortized for indexing and after the first search; it is fully visible on the first interactive query of a fresh `orbit` invocation. RPC serialization itself is sub-millisecond at phase-1 batch sizes (≤16 texts × ~512 tokens each); not a measurable contributor.

### 8.3 Default model quality is unmeasured for Orbit specifically

"BGE-small is fine" rests on published benchmarks (MTEB), not Orbit-specific recall numbers. Phase 1 deliberately does not ship an evaluation harness — building one in parallel with the feature is YAGNI before any user has hit a real recall failure. The cost is real: if BGE-small underperforms for Orbit's task corpus, we won't know until someone complains, and at that point we measure then. The `Embedder` trait + `model_id` PK column make swapping the default cheap whenever that day arrives ([3_vision.md §1.1](./3_vision.md)).

### 8.4 Brute-force scaling ceiling

Cosine over a full table scan stays sub-100ms at ~100K vectors. Phase-2 graph integration will push past that; the schema's forward compatibility with `sqlite-vec` is the planned upgrade path, but `sqlite-vec` is itself a loadable extension that may not be available in every distribution. The decision to revisit storage at phase 2 is in [4_decisions.md ADR-002](./4_decisions.md).

### 8.5 Multilingual content

BGE-small-en is English-tuned. Tasks written primarily in other languages will have weaker semantic recall. fastembed-rs supports multilingual models (e.g. paraphrase-multilingual-MiniLM); the model knob accommodates a swap, but the default ships English-tuned and that's a documented limitation, not a hidden one.

### 8.6 Privacy posture

All embeddings stay local. Task content never leaves the workspace. This is structural — there's no remote API in the loop — but worth stating explicitly because "AI feature" commonly implies "your data is going somewhere," and Orbit's semantic search does not.

---

## Task References

- [T20260510-3] — Design semantic search over task artifacts and graph (v2). The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

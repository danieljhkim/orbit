# Semantic Search — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-10

Semantic search is a local, offline-first retrieval layer over Orbit's task artifacts (phase 1) and, eventually, the knowledge-graph corpus (phase 2). Agents query it to find prior tasks by topic before adding duplicates; humans query it to recover work they remember by meaning rather than by literal substring. **Phase 1 ships in v1 as an opt-in feature**; phase 2 (graph integration) is reserved for a follow-up design once phase 1 is operational.

This document is the entry point. [2_design.md](./2_design.md) specifies the inference backend, vector storage, embedding strategy, and hybrid-retrieval pipeline; [3_vision.md](./3_vision.md) names open questions and prior work; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

The task store is already growing past the point where lexical recall is sufficient. Three concrete failure modes exist today:

1. **Duplicate tasks.** Agents create new tasks for problems that have already been worked on because `orbit task search` only matches literal substrings of titles and descriptions. A task titled "embed model latency degraded after Nomic swap" is invisible to a query for "slow inference."
2. **Lost prior work.** A human asks "didn't we have a task about token-counting heuristics?" and gets nothing because the original task used the phrase "context window estimation." The information is on disk, just not findable.
3. **Review-thread context loss.** Long-lived review threads accumulate decisions in comment bodies. Those decisions are unsearchable except by full text scan.

Lexical search via SQLite FTS5 (BM25) is part of the answer — it handles literal identifiers, error codes, and task IDs better than embeddings. But it misses the cases where the user's vocabulary doesn't match the document's. Semantic search via local embeddings handles that. The two are complementary, not competing, which is why phase 1 ships them together as a hybrid retrieval pipeline ([4_decisions.md ADR-004](./4_decisions.md)).

The constraint that shapes every other decision: **the default `orbit` install is single-binary, no-daemon, and no cloud dependency**. That rules out hosted embedding APIs and rules out an always-on inference daemon. Phase 1 keeps the default `orbit` binary slim by moving fastembed-rs into a separate `orbit-embed-companion` binary that the user opts into via `orbit semantic install` ([4_decisions.md ADR-001](./4_decisions.md), [ADR-005](./4_decisions.md)).

---

## 2. Core Concepts

### 2.1 Embedding backend (companion-binary architecture)

Two new crates land. `orbit-embed` is a small client library in the main `orbit` binary; it owns the `Embedder` trait, the JSON-RPC types, and a `SubprocessEmbedder` impl that talks to the companion. `orbit-embed-companion` is a separate binary built from its own crate; it depends on [fastembed-rs](https://github.com/Anush008/fastembed-rs) and ONNX Runtime and runs the actual inference. The main `orbit` binary has no fastembed dependency.

Users opt into semantic search by running `orbit semantic install [--model bge-small | minilm-l6 | nomic-v1.5]`, which downloads the platform-appropriate companion plus the chosen model into `~/.orbit/embed/`. Default model is BGE-small-en-v1.5 (384 dim, ~30MB). The trait abstraction exists so a future `orbit-embed-companion-candle` (or any other backend) can be swapped in without changing storage or retrieval. Airgapped operators have a manual-placement path described in [3_vision.md §1.2](./3_vision.md). The full backend selection rationale is in [4_decisions.md ADR-001](./4_decisions.md); the packaging decision is in [4_decisions.md ADR-005](./4_decisions.md).

### 2.2 Vector store

A new SQLite table `embeddings` stored alongside the existing task store. Each row holds `(source_kind, source_id, field, content_hash, model_id, dim, embedding BLOB)`. `source_kind` discriminates between rows that index task content and rows that will eventually index graph symbols; `field` distinguishes per-field embeddings within a task (one row each for `purpose`, `plan`, `comments_<idx>`, `review_<idx>`).

Phase 1 uses **brute-force cosine similarity** in Rust over the BLOBs. At task-corpus scale (low thousands of artifacts × small number of fields per task = tens of thousands of vectors at 384d), brute force is sub-millisecond per query and adds zero new dependencies. The on-disk format is forward-compatible with `sqlite-vec` should the graph corpus later push past brute-force scaling limits ([4_decisions.md ADR-002](./4_decisions.md)).

### 2.3 Hybrid retrieval

Queries run two retrievers in parallel: SQLite FTS5 (BM25) over a `tasks_fts` virtual table, and brute-force cosine over the `embeddings` table. The two ranked lists are fused via Reciprocal Rank Fusion (RRF, k=60) to produce the final ordering. RRF is an unweighted, parameter-light fuse that consistently outperforms either retriever alone in the published evaluation literature; it does not require either retriever's score to be calibrated to the other.

This is the single most important quality choice in the design. Pure semantic search loses on literal-identifier queries (function names, error codes, task IDs, file paths); pure lexical search loses on vocabulary-mismatch queries. RRF avoids picking one failure mode over the other ([4_decisions.md ADR-004](./4_decisions.md)).

### 2.4 Per-field embeddings

A task is indexed as multiple rows, one per field: `purpose`, `summary`, `plan`, each comment, each review-thread message. Search results return the best-matching field, and the result-formatting layer rolls multiple field hits on the same task into a single result with the highest-scoring field surfaced. This handles the BGE 512-token context limit naturally (most fields fit; long fields are chunked into multiple rows with a `chunk_idx`) and gives more precise results than concatenate-and-embed-once ([4_decisions.md ADR-003](./4_decisions.md)).

### 2.5 Phase boundary

Phase 1 covers tasks only. Phase 2 will add `source_kind = symbol` rows that embed graph nodes (module path + symbol name + docstring). The vector store schema is designed to accommodate this without migration, but phase 2 has its own design questions (which symbols, what to embed for them, how to keep embeddings fresh as code changes) that this folder does not pre-commit. Phase 2 lands as a separate task and a separate ADR cluster.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Inference backend choice (fastembed-rs) | [2_design.md §2](./2_design.md), [4_decisions.md ADR-001](./4_decisions.md) | [T20260510-3] |
| Companion-binary packaging + on-demand install | [2_design.md §2.2–§2.5](./2_design.md), [4_decisions.md ADR-005](./4_decisions.md) | [T20260510-3] |
| `orbit-embed` and `orbit-embed-companion` crate placement | [2_design.md §1](./2_design.md) | [T20260510-9] |
| Stdio JSON-RPC protocol | [2_design.md §2.3](./2_design.md) | [T20260510-9] |
| `embeddings` SQLite table schema | [2_design.md §3](./2_design.md), [4_decisions.md ADR-002](./4_decisions.md) | [T20260510-9] |
| Per-field embedding strategy | [2_design.md §4](./2_design.md), [4_decisions.md ADR-003](./4_decisions.md) | [T20260510-9] |
| FTS5 + cosine + RRF hybrid pipeline | [2_design.md §5](./2_design.md), [4_decisions.md ADR-004](./4_decisions.md) | [T20260510-10] |
| `orbit semantic install/uninstall` CLI | [2_design.md §6.1](./2_design.md) | [T20260510-9] |
| `orbit semantic search/related` CLI + MCP | [2_design.md §6](./2_design.md) | [T20260510-10] |
| Index-on-mutation + reindex command | [2_design.md §7](./2_design.md) | [T20260510-9] |
| Existing task store API | [crates/orbit-store/src/file/task_store/api.rs](../../../crates/orbit-store/src/file/task_store/api.rs) | — |
| Concerns & honest limitations | [2_design.md §8](./2_design.md) | [T20260510-3] |
| ADR log | [4_decisions.md](./4_decisions.md) | [T20260510-3] |
| Open questions, prior work | [3_vision.md](./3_vision.md) | [T20260510-3] |

---

## Task References

- [T20260510-3] — Design semantic search over task artifacts and graph (v2). The task that produced this folder.
- [T20260510-9] — Phase-1 foundation: `orbit-embed` + `orbit-embed-companion` crates, indexing pipeline, install command.
- [T20260510-10] — Phase-1 retrieval: hybrid query, CLI search/related, MCP tools.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---
summary: "Semantic Search — Decisions"
type: design
title: "Semantic Search — Decisions"
owner: claude
last_updated: 2026-07-26
status: Accepted
feature: orbit-search
doc_role: decisions
tags: ["orbit-search"]
---

# Semantic Search — Decisions

ADR-style log of non-obvious orbit-search decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. Every ADR names at least one cost. ADRs in this file carry status `Proposed` until the implementing task ships; they flip to `Accepted` with the implementing task ID at that point.

Historical note ([ORB-10458]): the entries listed below were authored with local IDs that had no record in the ADR store. They were allocated through `orbit.adr.add`, their narratives migrated into the store verbatim, and their headings rewritten to the allocated global ID. The original local IDs survive as `legacy_ids`, so prior citations still resolve via `orbit tool run orbit.adr.show --input '{"legacy_id":"<feature>/ADR-NNN"}'`. Backfilled here: `orbit-search/ADR-001` → ADR-0270, `orbit-search/ADR-002` → ADR-0271, `orbit-search/ADR-003` → ADR-0272, `orbit-search/ADR-004` → ADR-0273, `orbit-search/ADR-005` → ADR-0274, `orbit-search/ADR-006` → ADR-0275, `orbit-search/ADR-007` → ADR-0276, `orbit-search/ADR-008` → ADR-0277.

---

## ADR-0270 — fastembed-rs ONNX backend over Candle, llama.cpp, or external ollama

**Status:** Accepted · 2026-05 · [T20260510-3], [T20260510-9] · legacy_id: `orbit-search/ADR-001`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0270"}'`.

---

## ADR-0271 — Brute-force cosine over SQLite BLOBs; `sqlite-vec` reserved as phase-2 upgrade

**Status:** Accepted · 2026-05 · [T20260510-3], [T20260510-9] · legacy_id: `orbit-search/ADR-002`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0271"}'`.

---

## ADR-0272 — Per-field embeddings with chunked overflow, not whole-bundle concatenation

**Status:** Accepted · 2026-05 · [T20260510-3], [T20260510-9] · legacy_id: `orbit-search/ADR-003`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0272"}'`.

---

## ADR-0273 — Hybrid retrieval (FTS5 BM25 + cosine, fused via RRF) from day one

**Status:** Accepted · 2026-05 · [T20260510-3], [T20260510-9] · legacy_id: `orbit-search/ADR-004`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0273"}'`.

---

## ADR-0274 — Companion binary installed on demand, rather than bundled in `orbit`

**Status:** Accepted · 2026-05 · [T20260510-3], [T20260510-9] · legacy_id: `orbit-search/ADR-005`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0274"}'`.

---

## ADR-0275 — Workspace-local semantic DB separate from global audit/tool DB

**Status:** Accepted · 2026-05 · [T20260510-9] · legacy_id: `orbit-search/ADR-006`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0275"}'`.

---

## ADR-0276 — Semantic-search ownership relocated to `orbit-embed`

**Status:** Accepted · 2026-05 · [T20260510-20] · legacy_id: `orbit-search/ADR-007`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0276"}'`.

---

## ADR-0277 — Version-aware companion refresh and quiet background indexing

**Status:** Accepted · 2026-05 · [T20260510-26] · legacy_id: `orbit-search/ADR-008`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0277"}'`.

---

## ADR-0174 — Split lifecycle and query search namespaces

**Status:** Accepted · 2026-05-20 · [ORB-00196]

**Context.** `orbit semantic` mixed embedding-companion lifecycle (`install`, `uninstall`, `stats`, `index`) with user query verbs (`search`, `related`). The phase-1 search engine now owns both lexical and vector ranking, so leaving queries under `semantic` would make users choose an implementation detail before they search.

**Decision.** `orbit semantic` is only the lifecycle namespace for the local embedding companion. `orbit search` is the unified query surface; lexical ranking is the default, `--hybrid` opts into hybrid BM25 plus cosine for task vectors, and `--semantic <id>` performs cosine-neighbor lookup for indexed tasks.

**Consequences.**
- Establishes a precedent that lifecycle namespaces manage local subsystems while query namespaces describe what users are trying to do.
- `orbit semantic search`, `orbit semantic related`, and `orbit semantic reindex` are hard breaks with no shim because there are no known external consumers yet.
- Per-domain search commands stay untouched for phase 1; a later task decides whether they thin-wrap `orbit search`, demote to filters, or retire.
- At this phase, docs, learnings, and ADRs used lexical matching even when `--hybrid` was set; ADR-0180 later adds opt-in doc vectors while keeping learnings and ADRs lexical.

---

## ADR-0175 — Rename search mode and neighbor flags

**Status:** Superseded by ADR-0179 · 2026-05-21 · [ORB-00204]

**Context.** Phase 1 used the semantic name for the hybrid BM25 plus cosine mode toggle and a separate related-task flag for cosine-neighbor lookup. That inverted the intuitive reading of semantic search: users expect semantic plus an ID to mean nearest neighbors, while hybrid is the honest name for the ranking algorithm.

**Decision.** Rename the free-text ranking toggle to `--hybrid` / `hybrid: true` and rename task-neighbor lookup to `--semantic <id>` / `semantic: "<id>"`. Keep lexical search as the default and report JSON mode `hybrid` for hybrid free-text search and `neighbor` for cosine-only task-neighbor lookup.

**Consequences.**
- The CLI and MCP surfaces match user vocabulary before external consumers depend on the phase-1 names.
- Historical phase-1 audit payloads that carried `semantic: true` are orphaned by the hard break, matching the no-shim policy for this young surface.
- Documentation and packaged skills must distinguish the `orbit semantic` lifecycle command from the MCP `semantic: "<id>"` search parameter. ADR-0179 replaces the CLI flag form with `orbit search similar <id>`.
- Cost: Agents and docs written against phase 1 need a one-time rename sweep, and ORB-00202 may need a rebase because it edits adjacent search surfaces.
- Cost: historical audit event names `semantic.search` and `semantic.related` become orphaned event types, accepted because no external audit-history consumers exist yet.

---

## ADR-0176 — Consolidate per-domain search; cross-kind `--path` and `--tag` filters; learning list `--path` semantics flip

**Status:** Proposed · 2026-05-20 · [ORB-00202]

**Context.** After [ADR-0174] and [ADR-0175] consolidated `orbit search` as the unified query surface, the per-domain `task`, `docs`, and `learning` `search` subcommands of `orbit` became redundant for content-similarity queries. The `learning` variant in particular bundled three unrelated operations under one verb: substring search (content), path-glob applicability lookup (structural), and tag filter (structural). Agents pre-edit also need a single cross-kind command that answers *"given this file path, what tasks / learnings / ADRs apply here?"* — the context-pack query.

**Decision.** Hard-remove the per-domain `task`, `docs`, and `learning` `search` subcommands of `orbit` (CLI + MCP). Re-home their filters under the unified search surface: `--tag <T>` (AND semantics, repeatable, case-insensitive), `--all` (kind-aware status widener), `--status` (superseded by ADR-0179's `kind:value` syntax), and path applicability lookup (superseded by ADR-0179's `orbit search path <path>` CLI form; MCP keeps the `path` parameter). Add `orbit task list --path`; flip `orbit learning list --path` from exact-match to glob-containment. The old `--include-superseded` mental model from the retired per-domain doc surface is replaced by `orbit search --kind adr --all`. The structural-vs-content split — `search` for indexed content, `list` for structural filters — is enforced by the command layout.

**Consequences.**
- One mental model: `orbit search` queries indexed content, `orbit <kind> list` filters structural metadata.
- The agent context-pack query collapses to a single command (`orbit search path <file> --kind all`).
- Universal `--all` / `--status kind:value` vocabulary replaces the patchwork of kind-specific flags (`--include-superseded`).
- ORB-00203 fills the ADR filter branches by adding ADR `tags` and `paths`, without changing the public search surface.
- Cost: the `learning list --path` semantics flip is the only observable behavior change. Scripts calling `orbit learning list --path 'src/auth/**'` expecting exact-match scoped lookups will now also see paths *inside* that glob. The migration target for ex-`learning search --path` callers is unchanged because the new semantics match what that deleted command already did.
- Cost: during phase 2, ADR carried `--tag` and `--path` placeholders in two flag positions; ORB-00203 closes that gap by making those positions real filters.
- Cost: `AdrStatus` has no `Deprecated` variant, so `--all` adds `Superseded` only on ADRs. Asymmetric with task widening (which gets multiple terminal states); revisited if a deprecated state ever becomes load-bearing.
- Audit-row granularity is preserved by mapping `--kind` onto the `subcommand` field. Before consolidation, `orbit task search` / `orbit docs search` / `orbit learning search` produced distinct `(command, subcommand)` rows; after consolidation, `orbit search --kind X` produces `(command="search", subcommand="<kind>")` so downstream audit queries can still distinguish task / doc / learning / adr searches. Free-text content vs. structural lookup is not currently captured in the audit schema and is out of scope for this ADR.

---

## ADR-0179 — Split `orbit search` modes and require per-kind statuses

**Status:** Accepted · 2026-05-21 · [ORB-00205]

**Context.** ADR-0175 corrected the search flag names after phase 1, but the resulting CLI still mixed a positional query with mode flags and allowed flat status tokens whose meaning changed by corpus kind. The real alternatives were to keep extending that single-command flag matrix, or split the user-facing CLI modes before more corpora grow vector support.

**Decision.** Use three explicit CLI forms: `orbit search <query>` for free-text search, `orbit search similar <id>` for cosine-neighbor lookup, and `orbit search path <path>` for applicability lookup. Require `--status` values to use `kind:value` tokens such as `task:open`, `doc:active`, and `adr:proposed`. Remove the CLI field-selection and embedding-model flags, and remove the parallel MCP `field` and `embedding_model` parameters while keeping MCP `model` only as provenance.

**Consequences.**
- The CLI no longer has a top-level `<query | --semantic | --path>` trichotomy; each primary search operation has its own visible form.
- Status filters are unambiguous across task, doc, learning, and ADR corpora.
- MCP remains a parameterized tool surface, but it mirrors the reduced public parameter set and the same per-kind status parser.
- Cost: `similar` and `path` become reserved words immediately after `orbit search`; searching those literal words requires passing a quoted/free-text query with additional context.
- Cost: callers using the young mode flags, flat `--status`, the retired CLI field/model flags, MCP `field`, or MCP `embedding_model` surfaces must migrate with no compatibility shim.

---

## ADR-0180 — Doc corpus embeddings use `docs index` and opt-in hybrid search

**Status:** Accepted · 2026-05-21 · [ORB-00206]

**Context.** Doc search was lexical-only after [ORB-00202] unified the query surface, while the orbit-search store already had a `source_kind` discriminator that could hold docs. The alternatives were to keep semantic ranking deferred, add a separate docs search verb, or reuse the existing vector store behind the unified `orbit search --kind doc --hybrid` path.

**Decision.** Use `orbit docs index` as the explicit admin verb that embeds configured docs roots into `source_kind = "doc"` rows, and keep retrieval opt-in through `orbit search <query> --kind doc --hybrid`. Lexical doc search remains the default, ADRs stay lifecycle-owned and lexical-only, and `[docs.search].semantic_weight` tunes the blend without adding another CLI flag.

**Consequences.**
- `orbit docs index` shares the semantic companion, model catalog, and `embeddings` table with task vectors rather than creating a doc-specific store.
- The search crate owns doc field extraction and stale-source sweeping, but does not depend on orbit-core; core passes a small `DocEmbeddingSource`.
- Hybrid doc search falls back to lexical when the companion or doc rows are unavailable, preserving read-path ergonomics while making the admin indexing verb fail clearly.
- Cost: docs now have a manual freshness loop separate from task mutation indexing. Background docs indexing remains a future task.

---

## ADR-0244 — Expose unified search through a thin HTTP adapter

**Status:** Accepted · 2026-07-20 · [ORB-10304]

**Context.** Bridge needs hybrid Orbit search but can only proxy the dashboard HTTP surface. The alternatives were to keep reconstructing lexical results in Bridge, expose a generic tool-execution HTTP endpoint, or add a narrow search endpoint backed by the same runtime pipeline as the CLI.

**Decision.** Expose `GET /api/search` as a thin transport adapter over `OrbitRuntime::global_search`. The endpoint accepts the unified query, kind, status, tag, path, hybrid, and semantic parameters and returns the runtime response unchanged, including the effective mode and per-hit retriever rank breakdown. If hybrid infrastructure is unavailable, the shared runtime pipeline degrades to lexical so CLI and HTTP callers observe the same behavior.

**Consequences.**
- Bridge can proxy one authoritative endpoint instead of owning a second search implementation.
- CLI, tool, and HTTP search share filtering, ranking, result ordering, and fallback semantics.
- Cost: the unified search parameter names and serialized result shape become an HTTP compatibility contract; future search changes must preserve or deliberately version that surface.

---

## Task References

- [T20260510-3] — Design semantic search over task artifacts and graph (v2). The task that produced this folder.
- [T20260510-9] — Phase-1 semantic search foundation: orbit-embed + orbit-embed-companion + indexing pipeline. The task that accepted and implemented ADR-001 through ADR-006.
- [T20260510-20] — Refactor: relocate orbit-search ownership to orbit-embed (vector store + commands). The task that accepted and implemented ADR-007.
- [T20260510-26] — Make semantic companion install/update quiet and version-aware. The task that accepted and implemented ADR-008.
- [ORB-00196] — Split `orbit semantic` lifecycle from the unified `orbit search` query surface. The task that accepted and implemented ADR-0174.
- [ORB-00204] — Rename `orbit search` flags to `--hybrid` for free-text vector ranking and `--semantic <id>` for task-neighbor lookup. The task that accepted and implemented ADR-0175.
- [ORB-00202] — Consolidate per-domain search subcommands and add cross-kind `--path` / `--tag` filters. The task that proposed and implemented ADR-0176.
- [ORB-00203] — Add ADR envelope `tags` and `paths` so ADRs participate in cross-kind `--tag` / `--path` search filters.
- [ORB-00205] — Split `orbit search` into query / similar / path forms and require per-kind `--status` syntax. The task that accepted and implemented ADR-0179.
- [ORB-00206] — Add doc-corpus embeddings through `orbit docs index` and `orbit search --kind doc --hybrid`. The task that accepted and implemented ADR-0180.
- [ORB-10304] — Expose unified lexical, hybrid, and neighbor search through `GET /api/search`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

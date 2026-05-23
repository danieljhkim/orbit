---
title: "Orbit Docs — Decisions"
owner: claude
last_updated: 2026-05-21
status: Draft
feature: orbit-docs
doc_role: decisions
type: design
summary: "Orbit Docs — accepted ADRs: locked frontmatter schema, `.orbit/` vs `docs/` locating principle, ID-prefix dispatch, and doc embeddings indexing."
tags: [orbit-docs]
related_features: [orbit-docs]
related_artifacts: [ADR-0169, ADR-0170, ADR-0171, ADR-0180, ORB-00163, ORB-00206]
---

# Orbit Docs — Decisions

This file is the long-form narrative log for ADRs scoped to orbit-docs. Each entry's authoritative metadata (status, allocation, related_tasks) lives in the orbit-adr store at `.orbit/adrs/ADR-NNNN/adr.yaml`; this file is the prose explanation keyed on those global IDs.

ADR allocation is non-negotiable: the global ID is minted via `orbit.adr.add` *before* the heading appears here. See `docs/design/CONVENTIONS.md §4` for the rule and `docs/design/project-learnings/4_decisions.md` for a worked example of the discipline.

---

## ADR-0169 — Locked orbit-docs frontmatter schema

**Status:** Proposed · 2026-05 · [ORB-00163]

**Context.** Orbit ships three knowledge surfaces for agents (learnings, ADRs, design docs) and is adding a fourth, orbit-docs, as a storage-agnostic indexed-corpus surface for human-authored docs ([ORB-00163]). Without a constrained shape, the corpus drifts into the same per-feature ad-hoc Markdown that existed before, and the retrieval primitive becomes a substring search over arbitrary YAML, which is unrankable.

**Decision.** Numbered orbit-docs frontmatter is locked at exactly six fields: `type` (one of `design | pattern | context | glossary | runbook`, required), `summary` (non-empty single line, required), `tags` (string list, optional), `paths` (glob string list, optional), `related_features` (string list, optional), and `related_artifacts` (string list with ID-prefix dispatch — see [ADR-0171], optional). `type` and `summary` are strict; everything else is opportunistic. A tolerant indexer infers missing fields from directory and filename heuristics so legacy docs are discoverable without a forced migration.

**Consequences.**
- Retrieval-quality lever: ranking has predictable fields to score (`summary` text, `tags` exact, `type` exact). Future semantic ranking ([ORB-00168]) layers on top without renegotiating the schema.
- Indexer can be tolerant: dir-and-filename heuristics infer `type` and `summary` when frontmatter is absent, so the seed corpus works on day one ([ORB-00163] migrated 14 `4_decisions.md`, 12 sibling design docs, and 4 design-pattern docs).
- Cost: the schema is *closed*. Any seventh field (e.g. `last_updated`, `status`, `replaces`) requires another ADR. Plugin authors who want richer metadata must either piggyback on `tags` or argue for a schema extension. We chose closed-by-default over open-bag-of-fields specifically to keep the retrieval surface rankable.

---

## ADR-0170 — `.orbit/` for tool-managed artifacts; `docs/` for human-authored content

**Status:** Proposed · 2026-05 · [ORB-00163]

**Context.** Orbit accumulates persisted artifacts across two locations: `.orbit/` (tasks, learnings, friction, ADRs, audit DB, indexes, sessions, scoreboards) and `docs/` (design narratives, patterns, runbooks, glossaries). Before [ORB-00163] there was no written rule for which kind of artifact goes where, and the `.orbit/docs/` placement for orbit-docs was actively debated as the obvious-looking alternative.

**Decision.** The locating principle is now: **`.orbit/` is for tool-managed artifacts; `docs/` is for human-authored content.** Anything Orbit allocates IDs for, transitions through a lifecycle, indexes, or owns the storage shape of (ADR `adr.yaml`, learning YAML + SQLite index, task files, audit DB) lives under `.orbit/`. Anything authored by humans through PR review, with no Orbit lifecycle (designs, patterns, runbooks, glossaries), lives under `docs/`. Orbit-docs defaults its corpus root to `docs/` and the walker explicitly skips `.orbit/`. ADRs stay under `.orbit/adrs/` because they're tool-managed (allocation IDs, status transitions, supersede chains).

**Consequences.**
- Discoverability for new contributors: `docs/` is where they read; `.orbit/` is where tools write. Two locations, two roles, no confusion about which one to grep.
- Orbit-docs becomes a thin convention layer over `docs/` — no new on-disk store, no allocation IDs, no lifecycle. Authors keep ownership of layout (recommendation, not enforcement).
- The exclusion is a load-bearing invariant for the walker, not a soft suggestion: [ORB-00163] enforces it with a path-component check (`.orbit` anywhere in the relative path → skipped) and a regression test that points a tempdir root above a `.orbit/adrs/ADR-0001/body.md` and asserts the ADR is not surfaced.
- Cost: ADR corpus stays in a separate query surface from the docs corpus. An agent asking "what design context exists for feature X" currently has to query both `orbit.adr.*` and `orbit.docs.*`. Whether to fold ADRs into orbit-docs is the v2 design task [ORB-00169] — but that follow-up exists *because* we chose this strict boundary instead of letting orbit-docs own both corpora.

---

## ADR-0171 — ID-prefix dispatch for orbit-docs `related_artifacts`

**Status:** Proposed · 2026-05 · [ORB-00163]

**Context.** Orbit-docs frontmatter needs a way to cross-link from a doc to any other allocation-bearing artifact: a task (`ORB-NNNNN`), a learning (`L-NNNN`), a friction (`F<YYYY>-<MM>-<NNN>`), or an ADR (`ADR-NNNN`). The candidate shapes were (a) an array of `{type, id}` objects, (b) a single ambiguous `references` field, or (c) ID-prefix dispatch over a flat string array.

**Decision.** `related_artifacts` is a flat string array. The parser dispatches on the ID prefix to type the reference: `ORB-` → task, `L<digits>-<digits>` → learning, `F<digits>-<digits>-<digits>` → friction, `ADR-` → ADR. Unknown prefixes are a hard parse error (not silently kept as opaque strings).

**Consequences.**
- Frontmatter stays human-writable: `related_artifacts: [ORB-00163, ADR-0168]` is shorter and more skimmable than `[{type: task, id: ORB-00163}, {type: adr, id: ADR-0168}]`.
- The set of dispatchable prefixes is closed at parser-extension time, not at frontmatter-author time. Adding a new artifact kind (e.g. `M` for memory) requires editing the parser and adding a test, not negotiating with every doc author's frontmatter.
- Strict-unknown-prefix matters: silent acceptance of `XYZ-1` would let typos rot in the corpus undetected (`OBR-00163` instead of `ORB-00163`) and become broken cross-refs only at injection time. Hard erroring on parse forces the typo to surface at `orbit docs migrate`/`list`/`show` time, when there's a human reviewing.
- Cost: the prefix grammar is now load-bearing across orbit. The day Orbit changes task IDs from `ORB-NNNNN` to a different shape (say a UUID or a longer numeric range), the parser changes too — and so does any frontmatter already on disk. This is the same coupling cost the rest of orbit's ID conventions already pay; this ADR makes it explicit for orbit-docs's slice.

---

## ADR-0180 — Doc corpus embeddings use `docs index` and opt-in hybrid search

**Status:** Accepted · 2026-05-21 · [ORB-00206]

**Context.** Doc search was lexical-only after [ORB-00202] unified the query surface, while the orbit-search store already had a `source_kind` discriminator that could hold docs. The alternatives were to keep semantic ranking deferred, add a separate docs search verb, or reuse the existing vector store behind the unified `orbit search --kind doc --hybrid` path.

**Decision.** Use `orbit docs index` as the explicit admin verb that embeds configured docs roots into `source_kind = "doc"` rows, and keep retrieval opt-in through `orbit search <query> --kind doc --hybrid`. Lexical doc search remains the default, ADRs stay lifecycle-owned and lexical-only, and `[docs.search].semantic_weight` tunes the blend without adding another CLI flag.

**Consequences.**
- The old no-op docs indexing verb is retired rather than kept as a shim, so the docs lifecycle verb now matches `orbit semantic index`.
- Doc embeddings reuse orbit-search storage and companion model selection without adding an orbit-search to orbit-core dependency.
- Hybrid doc search can improve concept queries while preserving lexical fallback when the companion or doc rows are unavailable.
- Cost: the docs index becomes a second freshness loop next to task semantic indexing; operators must run `orbit docs index` after substantial doc moves or edits until background indexing exists.

---

## Task References

- [ORB-00163] — Introduce `orbit docs` indexed knowledge base and `orbit-docs` skill
- [ORB-00206] — Add doc-corpus embeddings: `orbit docs index` and hybrid scoring for `orbit search --kind doc`

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

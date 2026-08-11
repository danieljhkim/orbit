---
title: "Orbit Docs — Decisions"
owner: claude
last_updated: 2026-08-11
status: Draft
feature: orbit-docs
doc_role: decisions
type: design
summary: "Orbit Docs — accepted ADRs: locked frontmatter schema, `.orbit/` vs `docs/` locating principle, ID-prefix dispatch, and doc embeddings indexing."
tags: [orbit-docs]
related_features: [orbit-docs]
related_artifacts: [ADR-0169, ADR-0170, ADR-0171, ADR-0180, ORB-00163, ORB-00206]
last_validated: 2026-08-09
---

# Orbit Docs — Decisions

This file is the long-form narrative log for ADRs scoped to orbit-docs. Each entry's authoritative metadata (status, allocation, related_tasks) lives in the orbit-adr store at `.orbit/adrs/ADR-NNNN/adr.yaml`; this file is the prose explanation keyed on those global IDs.

ADR allocation is non-negotiable: the global ID is minted via `orbit.adr.add` *before* the heading appears here. See `docs/design/CONVENTIONS.md §4` for the rule and `docs/design/project-learnings/4_decisions.md` for a worked example of the discipline.

Historical note ([ORB-10479]): the entries listed below already held a global ADR allocation, but their store bodies were lost when the worktrees that authored them were reaped (see [F2026-07-163]). The narratives were restored into the store at their existing IDs — no ID was reallocated — and their headings reduced to pointer form. Restored here: [ADR-0169], [ADR-0170], [ADR-0171].

---

## ADR-0169 — Locked orbit-docs frontmatter schema

**Status:** Accepted · 2026-08-01 19:14:54.613724Z · [ORB-00163], [ORB-10479]
**Owner:** claude
**Created:** 2026-08-01 19:14:21.778846Z
**Last updated:** 2026-08-01 19:17:36.699047+00:00
**Related features:** `orbit-docs`
**Tags:** `orbit-docs`

**Context.** Orbit ships three knowledge surfaces for agents (learnings, ADRs, design docs) and is adding a fourth, orbit-docs, as a storage-agnostic indexed-corpus surface for human-authored docs ([ORB-00163]). Without a constrained shape, the corpus drifts into the same per-feature ad-hoc Markdown that existed before, and the retrieval primitive becomes a substring search over arbitrary YAML, which is unrankable.

**Decision.** Numbered orbit-docs frontmatter is locked at exactly six fields: `type` (one of `design | pattern | context | glossary | runbook`, required), `summary` (non-empty single line, required), `tags` (string list, optional), `paths` (glob string list, optional), `related_features` (string list, optional), and `related_artifacts` (string list with ID-prefix dispatch — see [ADR-0171], optional). `type` and `summary` are strict; everything else is opportunistic. A tolerant indexer infers missing fields from directory and filename heuristics so legacy docs are discoverable without a forced migration.

**Consequences.**
- Retrieval-quality lever: ranking has predictable fields to score (`summary` text, `tags` exact, `type` exact). Future semantic ranking ([ORB-00168]) layers on top without renegotiating the schema.
- Indexer can be tolerant: dir-and-filename heuristics infer `type` and `summary` when frontmatter is absent, so the seed corpus works on day one ([ORB-00163] migrated 14 `4_decisions.md`, 12 sibling design docs, and 4 design-pattern docs).
- Cost: the schema is *closed*. Any seventh field (e.g. `last_updated`, `status`, `replaces`) requires another ADR. Plugin authors who want richer metadata must either piggyback on `tags` or argue for a schema extension. We chose closed-by-default over open-bag-of-fields specifically to keep the retrieval surface rankable.

## ADR-0170 — `.orbit/` for tool-managed artifacts; `docs/` for human-authored content

**Status:** Accepted · 2026-08-01 19:14:58.079404Z · [ORB-00163], [ORB-10479]
**Owner:** claude
**Created:** 2026-08-01 19:14:56.604741Z
**Last updated:** 2026-08-01 19:17:40.466024Z
**Related features:** `orbit-docs`
**Tags:** `orbit-docs`

**Context.** Orbit accumulates persisted artifacts across two locations: `.orbit/` (tasks, learnings, friction, ADRs, audit DB, indexes, sessions, scoreboards) and `docs/` (design narratives, patterns, runbooks, glossaries). Before [ORB-00163] there was no written rule for which kind of artifact goes where, and the `.orbit/docs/` placement for orbit-docs was actively debated as the obvious-looking alternative.

**Decision.** The locating principle is now: **`.orbit/` is for tool-managed artifacts; `docs/` is for human-authored content.** Anything Orbit allocates IDs for, transitions through a lifecycle, indexes, or owns the storage shape of (ADR `adr.yaml`, learning YAML + SQLite index, task files, audit DB) lives under `.orbit/`. Anything authored by humans through PR review, with no Orbit lifecycle (designs, patterns, runbooks, glossaries), lives under `docs/`. Orbit-docs defaults its corpus root to `docs/` and the walker explicitly skips `.orbit/`. ADRs stay under `.orbit/adrs/` because they're tool-managed (allocation IDs, status transitions, supersede chains).

**Consequences.**
- Discoverability for new contributors: `docs/` is where they read; `.orbit/` is where tools write. Two locations, two roles, no confusion about which one to grep.
- Orbit-docs becomes a thin convention layer over `docs/` — no new on-disk store, no allocation IDs, no lifecycle. Authors keep ownership of layout (recommendation, not enforcement).
- The exclusion is a load-bearing invariant for the walker, not a soft suggestion: [ORB-00163] enforces it with a path-component check (`.orbit` anywhere in the relative path → skipped) and a regression test that points a tempdir root above a `.orbit/adrs/ADR-0001/body.md` and asserts the ADR is not surfaced.
- Cost: ADR corpus stays in a separate query surface from the docs corpus. An agent asking "what design context exists for feature X" currently has to query both `orbit.adr.*` and `orbit.docs.*`. Whether to fold ADRs into orbit-docs is the v2 design task [ORB-00169] — but that follow-up exists *because* we chose this strict boundary instead of letting orbit-docs own both corpora.

## ADR-0171 — ID-prefix dispatch for orbit-docs `related_artifacts`

**Status:** Accepted · 2026-08-01 19:15:01.117074Z · [ORB-00163], [ORB-10479]
**Owner:** claude
**Created:** 2026-08-01 19:14:59.800022Z
**Last updated:** 2026-08-01 19:17:43.600761Z
**Related features:** `orbit-docs`
**Tags:** `orbit-docs`

**Context.** Orbit-docs frontmatter needs a way to cross-link from a doc to any other allocation-bearing artifact: a task (`ORB-NNNNN`), a learning (`L-NNNN`), a friction (`F<YYYY>-<MM>-<NNN>`), or an ADR (`ADR-NNNN`). The candidate shapes were (a) an array of `{type, id}` objects, (b) a single ambiguous `references` field, or (c) ID-prefix dispatch over a flat string array.

**Decision.** `related_artifacts` is a flat string array. The parser dispatches on the ID prefix to type the reference: `ORB-` → task, `L<digits>-<digits>` → learning, `F<digits>-<digits>-<digits>` → friction, `ADR-` → ADR. Unknown prefixes are a hard parse error (not silently kept as opaque strings).

**Consequences.**
- Frontmatter stays human-writable: `related_artifacts: [ORB-00163, ADR-0168]` is shorter and more skimmable than `[{type: task, id: ORB-00163}, {type: adr, id: ADR-0168}]`.
- The set of dispatchable prefixes is closed at parser-extension time, not at frontmatter-author time. Adding a new artifact kind (e.g. `M` for memory) requires editing the parser and adding a test, not negotiating with every doc author's frontmatter.
- Strict-unknown-prefix matters: silent acceptance of `XYZ-1` would let typos rot in the corpus undetected (`OBR-00163` instead of `ORB-00163`) and become broken cross-refs only at injection time. Hard erroring on parse forces the typo to surface at `orbit docs migrate`/`list`/`show` time, when there's a human reviewing.
- Cost: the prefix grammar is now load-bearing across orbit. The day Orbit changes task IDs from `ORB-NNNNN` to a different shape (say a UUID or a longer numeric range), the parser changes too — and so does any frontmatter already on disk. This is the same coupling cost the rest of orbit's ID conventions already pay; this ADR makes it explicit for orbit-docs's slice.

## ADR-0180 — Doc corpus embeddings use docs index and opt-in hybrid search

**Status:** Accepted · 2026-05-21 02:07:03.161740Z · [ORB-00206]
**Owner:** codex
**Created:** 2026-05-21 02:06:58.051542Z
**Last updated:** 2026-05-21 02:10:33.828433Z
**Related features:** `orbit-search`, `orbit-docs`
**Tags:** `orbit-search`, `orbit-docs`, `embeddings`
**Paths:** `crates/orbit-core/src/command/docs.rs`, `crates/orbit-core/src/command/search.rs`, `crates/orbit-search/**`, `crates/orbit-cli/src/command/docs.rs`

### Context
Doc search was lexical-only after ORB-00202 unified the query surface, while the orbit-search store already had a source_kind discriminator that could hold docs. The alternatives were to keep semantic ranking deferred, add a separate docs search verb, or reuse the existing vector store behind the unified `orbit search --kind doc --hybrid` path.

### Decision
Use `orbit docs index` as the explicit admin verb that embeds configured docs roots into `source_kind = "doc"` rows, and keep retrieval opt-in through `orbit search <query> --kind doc --hybrid`. Lexical doc search remains the default, ADRs stay lifecycle-owned and lexical-only, and `[docs.search].semantic_weight` tunes the blend without adding another CLI flag.

### Consequences
- The old no-op docs indexing verb is retired rather than kept as a shim, so the docs lifecycle verb now matches `orbit semantic index`.
- Doc embeddings reuse orbit-search storage and companion model selection without adding an orbit-search to orbit-core dependency.
- Hybrid doc search can improve concept queries while preserving lexical fallback when the companion or doc rows are unavailable.
- Cost: the docs index becomes a second freshness loop next to task semantic indexing; operators must run `orbit docs index` after substantial doc moves or edits until background indexing exists.

## ADR-0153 — Allocate ADR IDs globally via orbit.adr.add before authoring feature decision entries

**Status:** Accepted · 2026-05-17 05:52:34.003993Z · [ORB-00098], [ORB-00019]
**Owner:** claude
**Created:** 2026-05-17 05:42:15.698605Z
**Last updated:** 2026-05-17 05:52:34.003993Z
**Related features:** `design-docs`, `adr-artifact`

### Context

The ADR v2 surface (`orbit.adr.*` tools + `.orbit/adrs/` store) shipped, but `docs/design/CONVENTIONS.md` §4 was not updated to require its use. As a result, agents editing `docs/design/<feature>/4_decisions.md` continued to follow the v1 markdown template and authored new local `## ADR-NNN` headings without allocating a global ID. The most recent example is `project-learnings/4_decisions.md` ADR-006 from ORB-00095; the global corpus does not know that decision exists. [ORB-00019] explicitly declined to settle the v1/v2 boundary (“do NOT collapse them; file a separate ADR”) — ORB-00098 is that follow-up.

Three policy options were on the table:

1. **Full v2 cutover.** New ADRs go *only* through `orbit.adr.add`; per-feature `4_decisions.md` becomes auto-generated from the store via the generator described in `docs/design/adr-artifact/2_design.md` §7.5. Hand-editing the markdown is disallowed.
2. **Dual surface with required mirroring (`legacy_ids` keyed on local 3-digit ADR-NNN).** Agents author the markdown ADR with a local 3-digit heading and also call `orbit.adr.add` with `legacy_ids: ["<feature>/ADR-NNN"]` in the same change.
3. **Markdown-first with sync tool.** Agents author markdown; a future `orbit adr sync` tool ingests new local ADRs into the store automatically.

Option 1 is the long-term destination but is not shippable in one task: the markdown generator does not exist, and retiring hand-edited `4_decisions.md` is a substantial behavior change. Option 3 introduces a second source of truth (markdown is canonical until sync runs) and indefinitely defers the global record — it weakens the corpus while pretending to feed it. Option 2 keeps the local 3-digit numbering scheme alive forever as the canonical heading, even though the heading number itself carries no information once a global ID exists.

The existing `docs/design/agent-families/4_decisions.md` already demonstrates a fourth option in the wild: the local heading **is** the global ID (`## ADR-0151 — ...`). It was allocated via `orbit.adr.add` first; the local file holds the long-form narrative and the global record holds metadata. No mirroring bookkeeping, no second numbering scheme.

### Decision

Go with the **global-ID-heading** stance:

- New ADRs MUST be allocated via `orbit.adr.add` *before* the local entry is written.
- The local heading in `docs/design/<feature>/4_decisions.md` uses the allocated global ID verbatim: `## ADR-NNNN — <title>` (4-digit, zero-padded).
- The local entry remains the long-form narrative log; the global record under `.orbit/adrs/` is the source of truth for ID, status, owner, `related_features`, and `related_tasks`.
- Existing local 3-digit headings (`activity-job/ADR-001`–`ADR-036`, etc.) are grandfathered. They may be backfilled opportunistically when a folder is being substantially edited; nothing forces it.
- `project-learnings/4_decisions.md` ADR-001–ADR-006 are backfilled in this task because they are recent and small enough to do cleanly.

This defers full v2 cutover (option 1) without blocking it: when the markdown generator ships, every `4_decisions.md` is already keyed on global IDs, so the generator can reconstruct files from the store without ID rewriting.

Lint enforcement of `[ADR-NNNN]` reference resolution was deliberately deferred from this task. The bet is that updating `CONVENTIONS.md` and the `orbit-adr` / `orbit-design` skill triggers is enough; if drift recurs, the lint becomes a follow-up.

### Consequences

- The local heading number carries information (it's the global ID), so cross-feature references can use the same `[ADR-NNNN]` syntax regardless of which folder the reader is in. No more `(activity-job, ADR-001) ≠ (project-learnings, ADR-001)` ambiguity for new entries.
- Agents have one clear instruction at authoring time: “call `orbit.adr.add` first, write the heading second.” Skill triggers in `orbit-adr` and `orbit-design` are updated to fire on “editing `4_decisions.md`” so the failure mode is caught at the right moment.
- The store is authoritative; `orbit.adr.list`, `orbit.adr.show`, and the future `orbit.adr.search` give honest answers without missing recent decisions.
- Cost: agents must remember the ordering. Without the deferred lint, there is no mechanical gate; the only enforcement is review. If the pattern drifts again, the lint becomes load-bearing.
- Cost: the local 4_decisions.md file ordering is no longer sequential per-folder. Entries are ordered by global ID, which interleaves with every other folder's allocations. Readers who relied on per-folder chronology lose that signal; the `created_at` line in the body preserves it.
- Cost: backfilling `project-learnings/ADR-001`–`ADR-006` rewrites the headings in `docs/design/project-learnings/4_decisions.md`. Existing citations in commits and tasks (e.g. `project-learnings/ADR-001`) still resolve through `legacy_ids`, but plain-text searches over the markdown file lose those numbers.

## ADR-0300 — Primary checkout is the sole authority for the documentation index

**Status:** Accepted · 2026-07-27 04:34:22.275158Z · [ORB-10504]
**Owner:** codex
**Created:** 2026-07-27 04:34:13.989332Z
**Last updated:** 2026-07-27 04:34:22.275158Z
**Related features:** `docs`
**Tags:** `docs`, `index`, `worktree`
**Paths:** `crates/orbit-core/src/command/docs/**`, `scripts/generate-doc-indexes.sh`, `docs/INDEX.md`

### Context
Orbit shares one workspace store and documentation index across linked Git worktrees. Letting `orbit docs list`, `show`, or `index` overlay the caller's worktree would make the same shared index return different content based on invocation directory and would introduce last-writer-wins races between concurrent task worktrees. The alternative was a per-worktree index or a worktree-first fallback such as ORB-10504.

### Decision
The primary checkout's resolved shared workspace root is the only authoritative source for the Orbit documentation corpus and index. `orbit docs index`, `list`, and `show` must not overlay or fall back to caller-worktree document content, and Orbit must not create a second per-worktree documentation index. The generated human-facing `docs/INDEX.md` likewise remains a single canonical artifact at the primary documentation root. Agents validate unmerged worktree documentation by reading the files directly and running the repository's frontmatter, generator, and freshness checks; those edits become visible through the documentation index only after they land in the primary checkout and the index is refreshed.

### Consequences
- Every caller observes one reproducible documentation corpus, independent of its current linked worktree.
- Concurrent task worktrees cannot overwrite or shadow one another in the shared docs index.
- Worktree validation uses source files and deterministic checks rather than treating a successful shared index refresh as proof of unmerged content.
- Cost: `orbit docs show` and search cannot preview unmerged worktree-only document edits; those edits must be inspected directly until they land in the primary checkout.

## Task References

- [ORB-00163] — Introduce `orbit docs` indexed knowledge base and `orbit-docs` skill
- [ORB-00206] — Add doc-corpus embeddings: `orbit docs index` and hybrid scoring for `orbit search --kind doc`

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

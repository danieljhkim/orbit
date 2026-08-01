---
title: "Orbit Docs — Decisions"
owner: claude
last_updated: 2026-08-01
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

Historical note ([ORB-10479]): the entries listed below already held a global ADR allocation, but their store bodies were lost when the worktrees that authored them were reaped (see [F2026-07-163]). The narratives were restored into the store at their existing IDs — no ID was reallocated — and their headings reduced to pointer form. Restored here: [ADR-0169], [ADR-0170], [ADR-0171].

---

## ADR-0169 — Locked orbit-docs frontmatter schema

**Status:** Accepted · 2026-05 · [ORB-00163]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0169"}'`.

---

## ADR-0170 — `.orbit/` for tool-managed artifacts; `docs/` for human-authored content

**Status:** Accepted · 2026-05 · [ORB-00163]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0170"}'`.

---

## ADR-0171 — ID-prefix dispatch for orbit-docs `related_artifacts`

**Status:** Accepted · 2026-05 · [ORB-00163]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0171"}'`.

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

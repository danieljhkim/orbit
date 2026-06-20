---
title: "Orbit Docs — Vision"
owner: claude
last_updated: 2026-05-21
status: Draft
feature: orbit-docs
doc_role: vision
type: design
summary: "Orbit Docs — open questions, the remaining roadmap (injection, ADR folding), and prior work in the agent-knowledge-base space."
tags: [orbit-docs]
related_features: [orbit-docs]
related_artifacts: [ORB-00164, ORB-00165, ORB-00166, ORB-00167, ORB-00168, ORB-00169, ORB-00206, ADR-0180]
---

# Orbit Docs — Vision

The v1 from [ORB-00163] is the corpus and the retrieval primitive; [ORB-00206] adds doc embeddings and opt-in hybrid ranking. Injection wiring and the ADR-folding question remain future work. This document names the open questions, the planned follow-up work, the prior art that shaped the design, and the dimensions on which orbit-docs differs from sibling tools.

---

## 1. Open Questions

### 1.1 Should `orbit-design` retire on the same cadence as orbit-docs adoption?

[ORB-00165] is filed but deliberately gated on three pre-flight conditions: at least two PRs using `orbit docs`, hook + task.show injection wired, and explicit team agreement that the 4-numbered layout is a recommendation rather than a rule. Retiring `orbit-design` too early forces a flag-day for authors who learned the old convention; retiring it too late leaves a duplicated retrieval surface (`orbit design list/show` vs `orbit docs list/show`) and a "which mental model do I use?" friction for new agents.

The harder sub-question: does `orbit-design` carry anything load-bearing that orbit-docs doesn't? Two candidates:

- **ADR earning rule.** `orbit-design` documents (and the docs/design conventions enforce) that ADR headings must be allocated via `orbit.adr.add` before they appear in `4_decisions.md`. This rule belongs to `orbit-adr`, not to a docs skill. Retiring `orbit-design` should not retire the rule.
- **`Last updated:` freshness assertion.** `orbit-design` asks authors to bump `last_updated:` when the doc materially changes. Orbit-docs has no such field; semantically, frontmatter `last_updated:` is just another tag. We chose not to lift this rule because (a) it's not enforceable without commit-graph integration, (b) it's noisy in practice (every cosmetic edit drives a debate about whether to bump), and (c) `git log -- <path>` answers the same question without an author-side assertion.

The third condition — explicit team agreement — is what makes this an open question rather than a queued task. The answer comes from usage, not from this design doc.

### 1.2 Should the docs corpus eventually fold ADRs?

[ORB-00169] is the design task. Three paths are named:

1. **Fold completely.** orbit-docs indexes `.orbit/adrs/` by translating `adr.yaml` to orbit-docs frontmatter at index time. ADR storage stays put. Single search surface; orbit-docs gains complexity for a corpus with a stricter lifecycle than it understands.
2. **Sibling indexes, unified search verb.** ADRs stay where they are with their own surface. The unified `orbit search` surface that landed in [ORB-00196] / [ORB-00202] already queries both via `--kind doc`, `--kind adr`, or `--kind all`. Clean separation; another surface to maintain.
3. **Status quo.** Skills document the boundary clearly and nudge agents to query both for design context. Zero new code; agents won't reliably query both.

The author's current bias is toward path 2 — keep the lifecycles separate, but provide a single retrieval entry point. But the choice should be made by [ORB-00169], not pre-committed here.

### 1.3 When do semantic embeddings start paying off?

[ORB-00168] is filed but priority-low. With ~100 docs today, BM25-ish substring + tag-exact matches the corpus shape well: most queries are "find the doc about RAII" (exact concept name) rather than "find the doc that explains anything resembling X" (semantic similarity).

The break-even point is roughly the size at which agents stop knowing the exact phrase they're looking for. Empirically, that's around 500 docs in our experience with similar systems, but the threshold is corpus-shape-dependent, not just count-dependent. A team that writes prose-heavy designs hits orbit-search payoff sooner than a team whose docs are mostly bullet lists and tables.

The right trigger is *retrieval-quality complaint*: when agents start saying "I can't find the doc I know exists," that's the signal to land [ORB-00168]. Not before.

### 1.4 What does injection latency cost the hook?

[ORB-00167] proposes a 2-doc cap (vs 3 for learnings) and gates the lookup behind `[hook].surface_docs = true`. The unanswered question: what's the actual latency budget?

The learning hook today walks `.orbit/learnings/` per Edit / Read / Write tool call and scores against scope-globs. With a few dozen learnings, that's ~5ms. Adding a parallel docs walk over 100 docs is another ~10-20ms (rough estimate). At ~30ms total per hook call, hook-fatigue starts to show up in agent flow.

The right answer is probably (a) share the walk between learning and doc lookup, and (b) cache the walked corpus for the duration of an `ORBIT_EXECUTION_ID`. Neither is in v1; both are noted in [ORB-00167].

### 1.5 Should there be doc IDs?

Currently docs are referenced by repo-relative path. That works for human authors and survives renames if `git log --follow` is run, but it's brittle as a stable cross-reference: when `docs/design/groundhog/2_design.md` becomes `docs/design/groundhog/architecture.md`, every `related_artifacts` entry pointing at the old path needs updating.

The alternative is to mint allocation IDs (`D<YYYYMMDD>-N` analogous to learnings, or a UUID stamped into frontmatter). This adds an `orbit.docs.add` allocation step, makes deletion a status flip rather than a file remove, and turns docs into a tool-managed artifact — at which point they belong under `.orbit/`, not `docs/`.

The tradeoff is straightforward: stability of cross-references vs. human-authored simplicity. We chose human-authored simplicity for v1. The day cross-references start breaking from doc renames is the day this question gets a task.

---

## 2. Prior Work

### 2.1 Orbit project learnings

[docs/design/project-learnings/](../project-learnings/) is the closest internal sibling. Both surfaces aim to elevate knowledge above per-agent memory into a shared queryable artifact, and both are intended for push-style injection eventually. They differ on:

| Dimension | Learnings | Orbit Docs |
|-----------|-----------|------------|
| Granularity | Sub-page rules with known failure modes | Multi-paragraph explanatory context |
| Storage | `.orbit/learnings/<id>.yaml` + SQLite index | `docs/**/*.md`, no on-disk store |
| Allocation | `orbit.learning.add` mints `L-NNNN` IDs | Author writes a file; no ID allocation |
| Lifecycle | `update`, `supersede`, `prune`, `upvote` | `add` a root, write files; no supersede flow |
| Discovery | Push-first (scope-glob injection) | Pull-first (search / show); push is downstream |
| Cross-references | `related_features`, `evidence` | `related_features`, `related_artifacts`, `paths` |

The boundary (rule-with-failure-mode vs. explanatory-context) is the load-bearing decision. Both surfaces being separate, with explicit cross-references via `related_artifacts: [L-NNNN]`, is the v1 shape.

### 2.2 Orbit ADRs

[.orbit/adrs/](../../../.orbit/adrs/) is the ADR artifact store. ADRs share with docs the "PR-reviewed Markdown" property but differ on lifecycle: ADRs have `proposed → accepted → superseded` and are tool-managed via `orbit.adr.add`. The locating principle ([ADR-0170]) puts them under `.orbit/adrs/`. Whether to fold them into orbit-docs is [ORB-00169].

### 2.3 Semantic search

[docs/design/orbit-search/](../orbit-search/) covers the embeddings infrastructure that orbit-search uses for tasks. [ORB-00168] extends that infrastructure to cover docs. The model and vector store stay the same; the index is a sibling of the task index.

### 2.4 Knowledge graph

[docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/) covers the code-symbol graph orbit-graph indexes. Orbit-docs is *not* a knowledge graph: it has no edges between docs except via `related_artifacts`. Cross-doc linking is plain Markdown relative paths. This is intentional — the knowledge graph is for code identifiers, not narrative content.

### 2.5 External: docs.rs, devdocs.io

The pattern of "Markdown corpus with metadata frontmatter, indexed for search" is well-trodden outside Orbit. docs.rs builds a per-crate doc index from rustdoc output; devdocs.io aggregates language and framework docs into a single search UI. Both inform the retrieval expectations agents bring: substring search should work, exact-name lookup should rank first, type filtering should be available.

The dimensions where Orbit Docs differs:

- **Author surface.** docs.rs/devdocs are read-only outputs of an upstream tool. Orbit Docs is author-edited Markdown checked into the repo.
- **Scope.** docs.rs/devdocs aggregate a known set of language/framework docs. Orbit Docs is unbounded — whatever the team writes.
- **Cross-corpus links.** docs.rs/devdocs link only to themselves. Orbit Docs links to tasks, learnings, ADRs, and friction reports via [ADR-0171].

### 2.6 External: Diátaxis framework

[Diátaxis](https://diataxis.fr/) categorizes docs into tutorials, how-to guides, reference, and explanation. Our `design | pattern | context | glossary | runbook` enum is a similar but coarser cut tuned to engineering-team output rather than user-facing documentation. The choice not to adopt Diátaxis verbatim is pragmatic: tutorials and how-to guides are not artifacts a team-internal docs corpus typically produces.

---

## 3. What May Be Distinctive

Three dimensions where orbit-docs is unusual relative to the prior work in §2:

### 3.1 Pull-first, retrieval-shaped

Most docs systems optimize for *humans browsing*. Orbit-docs is optimized for *agents retrieving*. The frontmatter schema treats `summary` as the retrieval cue, not the doc title. The walker is deterministic so JSON output is reproducible. The search ranker uses fields agents can query against (tags, type), not just full-text scoring. Even the recommended layout in the `orbit-docs` skill is framed as "what dir/filename will the tolerant indexer infer correctly?" rather than "what will a human reader find natural?"

This bias shows up in small places — `paths` as a glob list rather than a single value, `related_artifacts` as a flat array rather than typed objects — that look minor individually but compound into a corpus that an agent can crawl without an LLM in the loop.

### 3.2 Storage-agnostic, lifecycle-free

Most indexed-corpus tools own the storage shape (a database, a folder layout, an allocation scheme). Orbit-docs owns *one line of TOML* (`[docs].roots`) and nothing else on disk. Authors keep ownership of layout, files, and convention. The locking principle from [ADR-0170] makes this an enforceable boundary: tool-managed artifacts go under `.orbit/`, human-authored content goes under `docs/`, and `orbit-docs` is the convention layer between the two.

The cost is that orbit-docs cannot enforce things the storage-owner could enforce (frontmatter freshness, supersede chains, etc.). That's the v1 bet — that authors prefer un-enforced freedom over tool-enforced rigor for explanatory content.

### 3.3 Tolerant on read, strict on migrate

The strict / tolerant split lets the corpus be queryable on day one (tolerant inference handles legacy docs) while still offering a path to a canonical shape (the `migrate` verb backfills). Most schema-driven systems are one or the other: schemas-enforced-everywhere (Notion databases) or schema-suggested-but-never-enforced (loose Markdown directories). Tolerant-on-read with a strict-mode opt-in for new docs is the same shape `serde` brings to JSON deserialization, and it works for the same reason: the cost of strictness is moved to the author who *opts in* by writing frontmatter, not paid by the indexer on every read.

---

## 4. References

### 4.1 Orbit-internal

- [1_overview.md](./1_overview.md) — what orbit-docs is and the boundaries with learnings / ADRs
- [2_design.md](./2_design.md) — schema, walker, search, the six verbs
- [4_decisions.md](./4_decisions.md) — accepted ADRs
- [docs/design/project-learnings/](../project-learnings/) — sibling knowledge surface; rule-with-failure-mode shape
- [.orbit/adrs/](../../../.orbit/adrs/) — ADR artifact store; tool-managed lifecycle
- [docs/design/orbit-search/](../orbit-search/) — embeddings infrastructure; v2 target for [ORB-00168]

### 4.2 External

- [Diátaxis framework](https://diataxis.fr/) — alternative doc categorization (tutorials, how-to, reference, explanation)
- [docs.rs](https://docs.rs/) — read-only Rust doc index; informed retrieval expectations
- [devdocs.io](https://devdocs.io/) — multi-source doc aggregator; same lineage

---

## Task References

- [ORB-00163] — Introduce `orbit docs` indexed knowledge base and `orbit-docs` skill (shipped)
- [ORB-00164] — Harden orbit-docs internals: real diff, robust YAML edit, gitignore caching
- [ORB-00165] — Retire `orbit-design` skill in favor of `orbit-docs`
- [ORB-00166] — Wire `orbit docs` retrieval into `task.show --with-context` and `task.start`
- [ORB-00167] — Extend PreToolUse hook to surface relevant docs alongside learnings
- [ORB-00168] — Add semantic embeddings index for orbit-docs corpus (v2)
- [ORB-00169] — Design: fold `.orbit/adrs/` into the orbit-docs corpus (v2)

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

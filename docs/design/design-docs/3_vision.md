# Design Docs — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-17

Forward-looking concerns for the design-docs system. The current implementation ([2_design.md](./2_design.md)) ships strict layout, decay check, and a CLI + MCP tool surface. What is open: lint enforcement gaps, ADR migration to a queryable artifact, semantic search over design content, and tighter integration with the rest of the Orbit lifecycle (tasks → ADRs → docs).

---

## 1. Open Questions

### 1.1 Move ADRs into a queryable artifact store

The proposed [adr-artifact](../adr-artifact/) feature would lift ADRs out of per-feature `4_decisions.md` files into a first-class Orbit artifact with stable global IDs, structured lifecycle, supersession edges, and an `orbit.adr.*` tool surface. If that lands, this design-docs feature drops one of its four numbered files (or redefines `4_decisions.md` as a thin generated per-feature index). Open question: do the per-feature decision logs survive at all in v2, or does every ADR live only in the artifact store with feature association as a many-to-many edge?

### 1.2 Lint enforcement of frontmatter, Task References, and Cost lines

Today, [CONVENTIONS.md §11](../CONVENTIONS.md) lists two enforcement gaps as future work: every numbered doc must have valid frontmatter + a `Task References` section, and every ADR must have a `Cost:` line. Both are mechanical checks that could ship as part of `orbit design check` (e.g. additional finding categories: `MALFORMED_FRONTMATTER`, `MISSING_TASK_REFERENCES`, `ADR_MISSING_COST`). Open question: does this become a single command with stricter defaults, or does it earn its own subcommand (`orbit design lint`)?

### 1.3 Semantic search over design-doc content

Semantic search currently covers task artifacts ([docs/design/semantic-search/](../semantic-search/)). Extending it to design-doc bodies would let agents query "where do we explain X?" without grep. The 4-doc role split makes the query surface unusually well-shaped: search-by-role (e.g. "vision documents that mention sandboxing") is a natural cut. Open question: do design docs share the BLAKE3-deduped index used for tasks, or get their own source kind so role and feature filters are first-class?

### 1.4 Auto-bump `Last updated:` via pre-commit hook or CI

`Last updated:` is manual discipline today ([2_design.md §5.1](./2_design.md)). A pre-commit hook could detect "this commit touches `docs/design/<feature>/<doc>.md`" and require the `Last updated:` line in the same diff to be the commit date — refusing to commit otherwise. This catches the "I forgot to bump the date" failure without giving up the manual-anchor advantage from [4_decisions.md ADR-002](./4_decisions.md). Open question: is this annoying enough at PR time that authors disable it, defeating the point?

### 1.5 Reference extraction beyond regex

The decay check matches `(./...)` and `(../...)` link patterns ([2_design.md §7](./2_design.md)). A real markdown parser would also catch reference-style links and HTML anchors, and could distinguish "code reference" from "doc reference" (and weight them differently — a stale link to a sibling design doc is less load-bearing than a stale link to a renamed `crates/.../*.rs` file). Open question: is this worth the dependency cost on a markdown crate, or does the regex stay because the convention itself discourages exotic link styles?

### 1.6 Promote `4_decisions.md` `Status` line task IDs to typed lifecycle events

The Status line on an ADR currently embeds task IDs as plain text (`**Status:** Accepted · 2026-05 · [T20260419-2156]`). With ADRs as artifacts ([§1.1](#11-move-adrs-into-a-queryable-artifact-store)), each `Proposed → Accepted` flip becomes an audit event tied to the task that shipped the implementation. Open question: do these audit events backfill from existing markdown ADRs at migration time, or do we accept that pre-migration history is opaque?

### 1.7 Glossary as a first-class indexable artifact

`references/glossary.md` is recommended ([CONVENTIONS.md §5](../CONVENTIONS.md)) and follows a strict table format. With many feature folders accumulating glossaries, the same Orbit-specific term (`task_id`, `legacy_id`, `executor`) appears in multiple places with potentially diverging definitions. Open question: does a `orbit.design.glossary` tool surface a unified term index across all features, and what happens when two glossaries disagree?

---

## 2. Prior Work

### 2.1 Architecture Decision Record practice

- Michael Nygard's original 2011 essay on ADRs ([blog post](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)) defined the four-section structure (Title, Status, Context, Decision, Consequences) the Orbit ADR template extends. Orbit's modification: the mandatory `Cost:` line is not in the original; it exists because review-gating an ADR on "name a real cost" is the only thing reliably preventing trivial entries.
- Olaf Zimmermann and others extended ADRs into Sustainable Architectural Design Decisions; Adam Tornhill's *Software Design X-Rays* connects decision logs to behavioral code analysis. Both inform the rollup-fold mechanic in [CONVENTIONS.md §4a](../CONVENTIONS.md), which is original to Orbit but echoes Tornhill's "hotspot" reasoning about which decisions deserve to remain visible.

### 2.2 Documentation taxonomies

- Daniele Procida's [Diátaxis framework](https://diataxis.fr/) splits docs into tutorials / how-tos / reference / explanation. The 4-doc role split here (overview / design / vision / decisions) is closer to a *project-internal* analogue: `1_overview` is explanation, `2_design` is reference, `3_vision` is forward-looking explanation, `4_decisions` is the load-bearing decision log. Diátaxis itself was rejected as the literal layout because tutorials and how-tos belong at top-level project README, not per-feature.
- Living Documentation (Cyrille Martraire) emphasizes documentation co-located with code and refreshed mechanically; the decay check is a direct lift of that idea, scoped to design folders.

### 2.3 Frontmatter conventions

- Jekyll, Hugo, and other static-site generators popularized YAML frontmatter as a contract between docs and tooling. Orbit uses bold-label key-value pairs (`**Status:** Draft`) rather than YAML because the docs are read by humans first and parsed by tooling second; YAML at the top of every file is noisy in plain reading. The cost is a custom parser (one regex), which is bounded.

### 2.4 Decay-detection prior art

- Lightweight tools like [arc42 decay checker](https://github.com/arc42/arc42-template) and `vale` for prose linting tackle adjacent problems (structural completeness, prose style). None pin freshness to git timestamps of referenced source files. The closest precedent is Stripe's "doctest"-style runtime reference checks, which validate that documented API examples actually compile — different problem (correctness vs. freshness) but same instinct (mechanically tie docs to the artifact they describe).

### 2.5 Self-hosting documentation systems

- Several OSS projects (Rust RFC repo, Kubernetes KEPs) require contributors to follow strict templates and check against linters. The Orbit version is smaller-scope but harder-edged: authors are AI agents, not humans, so the convention has to be both machine-applicable (one tool call to scaffold) and machine-checkable (CI fails on drift). Human-oriented templates assume the author can context-switch into "now I write the doc" mode; agent-oriented templates need scaffolding so cheap that skipping it is harder than running it.

---

## 3. What May Be Distinctive

- **Decay anchored on a manual `Last updated:` field, not git mtime.** Every other freshness system the author surveyed uses some form of file-mtime or last-commit-date as the anchor. Orbit uses an author-asserted field because it carries an explicit semantic (*"I read this end-to-end and it still describes the system"*) that file mtime cannot. The cost is the manual-discipline failure mode; the gain is that cosmetic edits do not lie about freshness.
- **`Last updated:` decay is enforced in CI via the same binary the operator runs locally.** The decay check, the scaffolding, the per-feature inspection, and the convention rulebook all live in one repo and ship in one binary. There is no two-tool surface ("local linter + remote CI check") to drift apart.
- **Scaffolding is one MCP tool call.** Most documentation conventions ship as a CONTRIBUTING.md plus a `cookiecutter` template; the Orbit version makes scaffolding a first-class agent-callable operation that returns a typed summary of what was created. A non-trivial fraction of "agents that skipped writing the doc" are agents that did not know how to start.
- **The 3-of-3 ADR earning rule is unusual.** Most ADR practices are append-permissive — any decision can become an ADR, with after-the-fact pruning. Orbit gates entry on an explicit triple-test, which is more restrictive but keeps the log readable as a list of *load-bearing* decisions rather than *all* decisions.
- **Agent-attributed ownership.** The `Owner:` field is a single accountable agent family (`codex`, `claude`, `gemini`, or `grok`). The discipline is that one agent family is on the hook for keeping the folder coherent; cross-agent edits are welcome but the owner approves merges. This mirrors the broader Orbit posture that every write carries the identity of the agent that produced it.

---

## 4. References

### 4.1 Orbit-internal

- [docs/design/CONVENTIONS.md](../CONVENTIONS.md) — the convention rulebook this design folder implements and refines.
- [docs/design/adr-artifact/](../adr-artifact/) — proposal to lift ADRs into a queryable artifact store; closely linked to [§1.1](#11-move-adrs-into-a-queryable-artifact-store) and [§1.6](#16-promote-4_decisionsmd-status-line-task-ids-to-typed-lifecycle-events).
- [docs/design/semantic-search/](../semantic-search/) — current task-only semantic-search infrastructure, prerequisite for [§1.3](#13-semantic-search-over-design-doc-content).
- [ARCHITECTURE.md](../../../ARCHITECTURE.md) — crate layering and the `orbit-core` boundary that hosts the decay-check implementation.
- [crates/orbit-core/src/command/design.rs](../../../crates/orbit-core/src/command/design.rs) — implementation entry point.

### 4.2 External

- Michael Nygard, ["Documenting Architecture Decisions" (2011)](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions) — original ADR template.
- Daniele Procida, [Diátaxis framework](https://diataxis.fr/) — documentation taxonomy this folder split is loosely informed by.
- Cyrille Martraire, *Living Documentation by Design* (2019) — argument for mechanically-refreshed, code-co-located docs.
- Adam Tornhill, *Software Design X-Rays* (2018) — behavioral analysis informing the rollup-fold mechanic.
- [arc42 template](https://arc42.org/) — adjacent architectural-documentation template; influence on the section-role contract.

---

## Task References

- [ORB-00019] — Promoted the python decay checker to first-class `orbit design` tooling; this vision doc enumerates the open questions left after that promotion.
- [ORB-00090] — Aligned the ownership convention with family-based agent identity.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

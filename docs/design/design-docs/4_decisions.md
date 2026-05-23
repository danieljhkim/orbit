---
summary: "Design Docs — Decisions"
type: design
title: "Design Docs — Decisions"
owner: claude
last_updated: 2026-05-17
status: Draft
feature: design-docs
doc_role: decisions
tags: ["design-docs"]
---

# Design Docs — Decisions

Append-only ADR log for the design-docs feature. Each entry follows the template in [CONVENTIONS.md §4](../CONVENTIONS.md). Numbers are append-only; superseded entries stay in place with status updated. Every ADR cites at least one cost. New entries are allocated via `orbit.adr.add` *before* the local heading is written — see [CONVENTIONS.md §4](../CONVENTIONS.md) and the `orbit-adr` skill.

ADR-0158 through ADR-0161 retroactively document load-bearing decisions encoded in [CONVENTIONS.md](../CONVENTIONS.md) before this feature folder existed; the convention crystallized through review of [docs/design/](..) over time rather than a single shipping task, and the global records cite [ORB-00103] (the backfill that put them in the store) on the Status line. ADR-0162 and ADR-0163 document the [ORB-00019] promotion of the original tooling; ADR-0165 supersedes the per-PR freshness gate while keeping scaffolding and inspection.

Historical note: entries below were originally numbered ADR-001 through ADR-006 within this folder. They were allocated through `orbit.adr.add` and rewritten to global IDs per [ORB-00103]; the original local IDs survive as `legacy_ids` so prior citations still resolve via `orbit.adr.list --legacy-id=design-docs/ADR-NNN`.

---

## ADR-0158 — Four numbered docs with strict role separation

**Status:** Accepted · 2026-05 · [ORB-00103] · legacy_id: `design-docs/ADR-001`

**Context.** Per-feature design folders need a contract that makes cross-feature reading cheap. Free-form folders (every author picks their own structure) and single-doc folders (one big README) were both on the table. The first failure mode of free-form folders had already surfaced in early Orbit work: readers had to learn each folder's structure before they could find the decision history.

**Decision.** Every feature folder contains exactly four numbered markdown docs with fixed roles: `1_overview.md` (what and why), `2_design.md` (current implementation), `3_vision.md` (forward-looking), `4_decisions.md` (ADR log). A reader who learns the contract once can navigate any feature folder without re-orienting. Required-section lists per file ([CONVENTIONS.md §3](../CONVENTIONS.md)) lock in section order so that "open `2_design.md` and read the mechanism numbered §3" is a stable instruction.

**Consequences.**

- Cross-feature reading is cheap: the same file name means the same thing everywhere.
- Authors who would have written one doc are forced to write four, even when the feature is small.
- The required-section contract is checkable; today by review, eventually by lint ([3_vision.md §1.2](./3_vision.md)).
- Cost: tiny features (a single mechanism with no open questions and no real decisions) end up with shallow `3_vision.md` and `4_decisions.md` files. The fix is not "drop the file" but "the feature was probably too small for its own folder" — promote the work into an existing folder instead. This pressure is sometimes ignored, leaving a few thin folders.

---

## ADR-0159 — `Last updated:` is the author assertion, not git mtime

**Status:** Accepted · 2026-05 · [ORB-00103] · legacy_id: `design-docs/ADR-002`

**Context.** The original freshness tooling needed a per-doc timestamp to compare against `git log` of referenced source files. Two anchors were on the table: (a) parse `Last updated:` from frontmatter, requiring authors to bump it manually; (b) use `git log -1 --format=%cs -- <doc>.md` directly. Option (b) eliminates the manual-discipline failure mode; option (a) carries an explicit author assertion.

**Decision.** Use the `Last updated:` field. The author updates it manually whenever the doc body changes substantively; cosmetic edits (typo fixes, link reflows, whitespace) intentionally do *not* bump it. Tooling and review treat the field as the author's assertion.

**Consequences.**

- The freshness signal carries an explicit semantic: "the author has read this doc end-to-end and asserts it still describes the system." `git log` of the doc cannot carry that semantic.
- Cosmetic-only PRs do not falsely reset the staleness clock for a six-month-stale doc.
- The discipline is enforceable by review (and eventually a pre-commit hook, [3_vision.md §1.4](./3_vision.md)) but not by automation itself.
- Cost: an author who forgets to bump the date ships a doc that looks fresh until the next reviewer notices. This is the dominant failure mode of the system today; it is accepted as the price of the explicit-assertion semantic.

---

## ADR-0160 — ADRs earned by 3-of-3 rule, not append-permissive

**Status:** Accepted · 2026-05 · [ORB-00103] · legacy_id: `design-docs/ADR-003`

**Context.** Most ADR practices are append-permissive: any decision can become an ADR, and pruning is after-the-fact. Append-permissive logs grow fast and bury load-bearing decisions in trivia ("we picked this library version," "we used `?` instead of `match`"). The question was whether to inherit the permissive default or gate entry.

**Decision.** A decision earns an ADR only when *all three* hold: (1) a real alternative was on the table, (2) the choice constrains future work, (3) the cost is non-trivial and not inferable from the decision itself. If only one or two hold, the decision lives in `2_design.md` prose, as a row in an existing ADR's table, or as a task-ID citation on the parent ADR's status line. Every ADR must include at least one bullet labeled `Cost:`. See [CONVENTIONS.md §4](../CONVENTIONS.md).

**Consequences.**

- The log stays readable as a list of *load-bearing* decisions; readers can scan the titles and grasp the architectural shape of the feature.
- Cross-review enforces the rule; rejected ADRs are downgraded to design-doc prose or removed.
- Cost: the rule is judgmental. Reasonable agents disagree on whether a given decision crosses the bar, and reviews occasionally re-litigate the rule itself rather than the substance. There is no automated check today; lint cannot verify "is this decision load-bearing" because the question is semantic. The rollup-fold mechanic ([CONVENTIONS.md §4a](../CONVENTIONS.md)) is the maintenance escape hatch when a cluster of accepted ADRs turns out to instantiate the same underlying choice.

---

## ADR-0161 — Folder names lowercase, hyphenated, singular

**Status:** Accepted · 2026-05 · [ORB-00103] · legacy_id: `design-docs/ADR-004`

**Context.** Folder naming is a low-stakes choice in isolation but high-stakes once cross-links accumulate. Options on the table: `PascalCase`, `snake_case`, `kebab-case`; singular vs plural; whether to allow nested folders.

**Decision.** Folder names are lowercase, hyphenated (`kebab-case`), singular: `knowledge-graph` not `KnowledgeGraph`, `policy-sandbox` not `policy_sandbox`, `task-artifacts` (which deliberately reads as "the family of task artifacts" but stays singular as a folder concept). No nesting; every feature is a sibling under `docs/design/`. Folder names matching `_*` (e.g. `_archive/`) are reserved for retired-feature storage and are skipped by tooling.

**Consequences.**

- Cross-link paths are uniform and predictable.
- The retirement path (`mv docs/design/foo docs/design/_archive/foo`) is a one-line operation that tooling respects automatically.
- Cost: renaming a feature folder breaks every link to its docs. The fix is mechanical (grep + sed) but requires touching every doc that referenced the old name. There is no rename redirect mechanism. Practically, folder names are very rarely renamed once established; the cost has been paid twice in two years.

---

## ADR-0162 — Promote python freshness checker to first-class `orbit design` CLI + MCP

**Status:** Superseded by ADR-0165 · 2026-05 · [ORB-00019] · [ORB-00103] · [ORB-00112] · legacy_id: `design-docs/ADR-005`

**Context.** The original freshness checker shipped as a small python script wrapped by a Make target. Three properties of that placement bothered enough to file ORB-00019: (a) agents driving Orbit through MCP could not invoke the checker without shelling out, (b) the parser duplicated logic that belonged with the rest of the design-doc tooling (which was about to grow scaffolding and inspection), and (c) the script was not exercised by Orbit's own integration tests, so a parser bug could ship undetected. Three options were on the table: keep python and just expose it through MCP via a shim; rewrite in Rust as a CLI only; rewrite in Rust with both a CLI and an MCP tool surface.

**Decision.** Rewrite the checker in Rust as part of `orbit-core::command::design` and expose it beside the design-doc scaffolding and inspection surface. Keep the legacy script path as a thin compatibility wrapper during the promotion.

**Consequences.**

- Agents driving Orbit through MCP can scaffold and inspect design folders without shelling out, which makes "skip the docs" the harder choice in agent-driven workflows.
- The freshness-check logic gets covered by the workspace test suite; output equivalence with the python script was verified end-to-end before the python code was deleted.
- The init/list/show surface enables future tooling (lint, semantic search, glossary index, [3_vision.md §1.2](./3_vision.md)–[§1.6](./3_vision.md)) to extend a Rust API rather than fork a python script.
- Cost: more code to maintain than 117 lines of python (~700 lines of Rust across `orbit-core::command::design`, the CLI shim, the MCP tool registry entries, and the dispatch). The boundary between `orbit-core` and `orbit-cli` had to be plumbed for the new command. ADR-0165 later removes the checker-specific portion of this cost after the signal proved misleading.

---

## ADR-0163 — `init_feature` refuses to clobber an existing folder

**Status:** Accepted · 2026-05 · [ORB-00019] · [ORB-00103] · legacy_id: `design-docs/ADR-006`

**Context.** `orbit.design.init` accepts a feature name and scaffolds a folder. The question was whether to overwrite an existing folder (with or without a `--force` flag), error on existing, or merge into existing (write only the missing files). Overwrite is convenient but destructive — a re-run after editing would silently undo the author's work. Merge is convenient but produces an inconsistent state when a folder is partially scaffolded by hand and partially by tool.

**Decision.** `init_feature` errors with a typed `InvalidInput` when the target folder already exists. There is no `--force` or `--merge` flag. To re-scaffold, the author must move or delete the existing folder explicitly.

**Consequences.**

- A re-run after editing cannot silently destroy work.
- The init operation has a clean precondition (folder absent); diagnosis of failures is a single check.
- Cost: an author whose first scaffold was a typo (e.g. they ran `init` with the wrong feature name, then noticed) has to delete the wrong folder before re-running. There is no in-tool fix. This has not been painful in practice — typos surface immediately because the response shows the path — but the lack of a `--force` is occasionally requested.

---

## ADR-0165 — Remove per-PR design-doc decay checks

**Status:** Accepted · 2026-05 · [ORB-00112]

**Context.** The `orbit design check` CLI, `orbit.design.check` MCP tool, `make check-design-docs` target, and `scripts/check_design_doc_decay.py` wrapper compared each design doc's `Last updated:` value with referenced code timestamps. ORB-00110 exposed release-blocking false positives during v0.6.0 promotion; ORB-00111 would have made the implementation deterministic by rebasing on git committer dates, but most code edits still do not invalidate prose claims. The worse behavior was social: the gate trained agents to bump dates instead of re-reading docs.

**Decision.** Delete the per-PR decay-check surface and keep the useful design-doc tooling: `init`, `list`, and `show` for CLI and MCP callers. The replacement gate is the existing same-PR update rule plus code review against [CONVENTIONS.md](../CONVENTIONS.md).

**Consequences.**

- ORB-00111 is superseded because determinizing the old check would preserve the wrong signal.
- The design-doc tool surface becomes smaller and easier to explain: scaffold, list, show.
- Cost: Orbit loses an automated stale-doc check, but there is no documented case where it caught a real bug that the same-PR update rule and review would have missed.

**Alternatives considered.**

- **Content-level structural lint.** Still potentially useful, but it should validate explicit structure (frontmatter, Task References, ADR Cost lines) under a lint-shaped command rather than timestamp freshness.
- **Periodic audit.** A slower human or agent audit cadence better matches design-doc decay, which tends to unfold over weeks or months rather than within one PR.

---

## Task References

- [ORB-00006] — Refresh of ARCHITECTURE.md and existing design folders that produced the layout codified in [ADR-0158].
- [ORB-00019] — Promotion of the freshness checker and scaffolder into first-class Rust + MCP tooling, documented in [ADR-0162] and [ADR-0163].
- [ORB-00090] — Aligned design-doc ownership metadata with family-based agent identity.
- [ORB-00103] — Backfilled this folder's ADR-001 through ADR-006 into the global store with `legacy_ids`; rewrote local headings to global IDs.
- [ORB-00112] — Removed the per-PR freshness gate, superseding [ADR-0162] with [ADR-0165].

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

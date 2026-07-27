---
title: Design Doc Conventions
owner: daniel
last_updated: 2026-07-26
last_validated: 2026-07-26
status: Accepted
---

# Design Doc Conventions

Recommended conventions that feature leads follow when writing and maintaining design docs under `docs/design/<feature>/`. The goal is a set of feature folders that read as one coherent documentation system regardless of which agent authored them. These are recommendations (not hard rules enforced by tooling; `orbit-design` has been retired in favor of the more tolerant docs surface — see the `orbit-search` skill).

This doc is itself the source of truth for the conventions. When a convention changes, update this doc and then update existing feature folders to match — do not silently diverge.

---

## 1. Folder Layout (per feature, recommended)

```
docs/design/<feature>/
├── 1_overview.md       recommended — what and why
├── 2_design.md         recommended — current implementation
├── 3_vision.md         recommended — forward-looking
├── 4_decisions.md      recommended — ADR pointer index
├── specs/              recommended folder; may be empty initially
│   └── <mechanism>.md  one mechanism per file
└── references/         recommended folder; may be empty initially
    └── glossary.md     recommended; other lookup-style docs allowed
```

- Folder name: lowercase, hyphenated, singular (`knowledge-graph`, `host-registry`).
- No `README.md`, `roadmap.md`, `changelog.md`, `tutorial.md` at this level.
- No top-level narrative files outside the numbered four (`1_`–`4_`). Existing folders may vary; new work should prefer the layout for coherence.

**Starting a new feature?** Copy the ready-made scaffold, then fill the placeholders:

```sh
cp -r docs/design/_templates docs/design/<feature>
mv docs/design/<feature>/specs/_mechanism.md docs/design/<feature>/specs/<mechanism>.md
```

The [`_templates/`](./_templates/) files carry the required frontmatter and section skeletons. They are the canonical copy source — the sections below describe only the *rules*, and point at the template they instantiate instead of repeating the boilerplate.

---

## 2. Required Frontmatter (all numbered docs)

Every numbered design doc starts with the YAML frontmatter carried by the [`_templates/`](./_templates/) `N_*.md` files — copy it and fill the placeholders. The fields:

- `title` mirrors the H1 verbatim.
- `owner` is the accountable agent family, not a committer list or full model string.
- `last_updated` is the calendar date of the last meaningful content change. Trivial reformat commits should not reset it.
- `status` is `Draft` until the doc is approved by the feature lead, then `Accepted`. It moves back to `Draft` if a structural rewrite is in flight.
- `feature` is the folder slug (e.g. `host-registry`, `knowledge-graph`). Lets tooling group docs by feature without parsing paths.
- `doc_role` is one of `overview`, `design`, `vision`, `decisions` — corresponds 1:1 with the filename prefix `1_`/`2_`/`3_`/`4_`.

The template frontmatter also carries the orbit-docs retrieval fields (`type`, `summary`, `tags`, `paths`, `related_features`, `related_artifacts`) so the doc is indexable on day one. `type` and `summary` are required by the strict parser. `summary` must be a non-empty single line. `related_artifacts` accepts `ORB-NNNNN`, `L-NNNN`, `FYYYY-MM-NNN`, and `ADR-NNNN` strings. The tolerant indexer infers these fields for legacy design docs and pattern docs, but new docs should write them explicitly. The docs indexer does not index `.orbit/`; ADR bodies remain owned by the ADR tool surface (see the `orbit-knowledge` skill).

---

## 3. Required Sections per Numbered Doc

| File | Required sections (in order) |
|------|------------------------------|
| **1_overview.md** | Elevator paragraph · §1 Motivation · §2 Core Concepts · §3 At a Glance (table: concern → file → task) · Task References |
| **2_design.md** | Scope paragraph · mechanism sections (variable count, numbered) · §N Concerns & Honest Limitations (mandatory last section) · Task References |
| **3_vision.md** | Scope paragraph · §1 Open Questions (numbered) · §2 Prior Work (subsections by category) · §3 What May Be Distinctive · §4 References (Orbit-internal + External) · Task References |
| **4_decisions.md** | Pointer-index explainer (including the `orbit.adr.show` command) · ADR pointers in ascending number order |

Every numbered doc ends with a **Task References** section listing only the task IDs cited in that doc, plus the line:

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---

## 4. ADR Template (strict)

**Allocation order is non-negotiable.** Before adding an ADR pointer, allocate the global ID via `orbit.adr.add` (see the `orbit-knowledge` skill and [ADR-0153]). The local pointer then uses the allocated global ID verbatim — never invent a four-digit number that "looks global." The store is the source of truth for the title, body, ID, status, owner, `related_features`, and `related_tasks`; `4_decisions.md` is an ordered pointer index, not a second narrative log.

Copy the pointer from [`_templates/4_decisions.md`](./_templates/4_decisions.md): `- **ADR-NNNN — <title>** — <status>.` Readers retrieve the authoritative `Context` / `Decision` / `Consequences` body with `orbit tool run orbit.adr.show --input '{"id":"ADR-NNNN"}'`. The store body, not the local index, must carry the ADR's mandatory `Cost:` consequence.

Rules:

- **Allocate first.** `orbit.adr.add` returns the global `ADR-NNNN` (4-digit, zero-padded). That ID is your local pointer. Never hand-author an `ADR-NNNN` pointer without an allocation behind it. Bypassing this is the failure mode [ORB-00098] resolved; see [ADR-0153].
- **Inline cross-references** use the global ID (`[ADR-0042]`), resolvable via `orbit tool run orbit.adr.show --input '{"id":"ADR-0042"}'`.
- Numbers are append-only; superseded records stay in the index with their store status.
- `Proposed` is allowed only before the relevant task ships. Flip the store record to `Accepted` via `orbit.adr.update` when it lands, then refresh the index status on its next edit.
- Every ADR body must cite at least one cost. No cost = the decision wasn't real.
- **Legacy 3-digit headings.** Existing local 3-digit headings (`## ADR-NNN`) authored before the global-ID convention are grandfathered narrative records until separately backfilled. When backfilled, allocate the global ID first, preserve the original local ID as a `legacy_id` in the store record, verify that the store body carries the narrative, and replace the local body with its global pointer. See `docs/design/project-learnings/4_decisions.md` and `docs/design/agent-families/4_decisions.md` for worked examples.

An entry earns its own ADR only if **all three** hold:

1. **Real alternative.** A different choice was on the table and would have produced a materially different design — not "we did the obvious next instance of an existing pattern."
2. **Forward constraint.** The decision shapes future work, rules out a class of approaches, or imposes a non-trivial tradeoff readers will need to know about months later.
3. **Non-trivial cost.** The cost line names something a reader couldn't infer from the decision itself ("we now depend on grammar X" is trivial; "stable_id reallocates every object hash on first rebuild" is not).

If only one or two hold, the decision belongs in `2_design.md` prose, as a row in an existing ADR's table, or — for plain-instance work — as a task-ID citation on the parent ADR's Status line.

---

## 4a. Rollup ADRs

When a cluster of accepted ADRs all instantiate the same underlying decision (e.g. "added language X to the tree-sitter extractor set"), the cluster may be folded into a single rollup ADR:

- The rollup either reuses the parent ADR's number with an expanded body and a per-instance table, or claims a new number that lists the cluster.
- Each folded entry stays in the index with the store status (for example, `Superseded by ADR-NNN (folded)`).
- The rollup's store body must preserve every Cost line from the folded entries that doesn't duplicate a cost already named.
- Compaction is a normal maintenance operation, not an emergency cleanup. Owners should fold a cluster when the third instance lands, not the tenth.

---

## 5. Glossary Format

Copy [`_templates/references/glossary.md`](./_templates/references/glossary.md): an intro paragraph (scope and deliberate exclusions) followed by an alphabetized `Term | Meaning` table.

Rules:

- Alphabetized.
- Orbit-specific vocabulary only. Standard industry terms (hunk, blob, content-addressed, TTL) are excluded by default unless the feature gives them a specific meaning.
- Every entry references the doc where the term is used, so definitions don't drift from implementation.

---

## 6. Spec Format (`specs/<mechanism>.md`)

Copy [`_templates/specs/_mechanism.md`](./_templates/specs/_mechanism.md): a one-paragraph contract statement, a **Why This Exists** section, mechanism-specific sections, and an optional **Agent Signature**.

A spec is **prescriptive**. It names invariants ("writes do not fall back"), failure modes ("detached HEAD errors"), and migration paths. It is *not* a design-rationale doc — rationale lives in the ADR store and is indexed by `4_decisions.md`.

---

## 7. Status Lifecycle (per doc)

- **Draft** — pre-first-review. Owner is still shaping it.
- **Accepted** — reviewed, approved, load-bearing.

There is no `Deprecated` status at the doc level. If the feature is retired, archive the entire folder under `docs/design/_archive/<feature>/` and annotate the first line of `1_overview.md`.

---

## 8. Cross-link Conventions

- Relative paths only, always with `./` or `../` prefix: `[foo](./foo.md)`, `[bar](../other/bar.md)`.
- Never link a task ID — `[ORB-00042]` stays as plain bracketed text. It's searchable via `git log --grep=` regardless of where tasks are stored.
- Section references use full paths: `[2_design.md §6.3]`, not a bare `§6.3` from a sibling doc.

---

## 9. Task ID Citation Format

- Inline: plain bracketed text `[ORB-00042]`.
- In ADRs: on the status line after the date.
- Never cite a task without naming what that task did — `([ORB-00042])` alone is opaque; always give a verb phrase.

---

## 10. What NOT to Create

| Anti-pattern | Why |
|--------------|-----|
| `README.md` at the feature folder | Duplicates `1_overview.md` |
| `roadmap.md` | Belongs in Orbit task system |
| `changelog.md` | Covered by git history + task IDs |
| `tutorial.md` | Belongs at top-level project README |
| Task-artifact mirrors in `references/` | ADRs should absorb the "why"; rot risk otherwise |
| Top-level doc outside the numbered four | If it's important, it belongs inside one of them |

---

## 11. Enforcement

These are recommendations, not mechanically enforced by `orbit-design` (retired) or the docs indexer. The tolerant indexer (see the `orbit-search` skill) accepts both strict numbered design folders and free-form docs.

Two mechanical checks worth adding later (as optional lints, never blocking):

1. Lint: every numbered doc has required frontmatter + Task References section.
2. Lint: every ADR has a Cost line.

Until those exist: cross-review and author judgment are the quality mechanism. When one agent reviews the other's docs, the reviewer treats this doc as a checklist and gives feedback on deviations; the author decides whether the deviation is justified for that folder.

---

## 12. Ownership

The `Lead` value mirrors each feature folder's frontmatter `owner:` field and
uses the canonical agent family (`codex`, `claude`, `gemini`, or `grok`).
Retired features stay listed with their `_archive/` path as a historical record.

| Feature | Folder | Lead |
|---------|--------|------|
| Activity / Job | [docs/design/activity-job/](./activity-job/) | codex |
| Agent Families | [docs/design/agent-families/](./agent-families/) | grok |
| Auditability | [docs/design/auditability/](./auditability/) | codex |
| Executors | [docs/design/executors/](./executors/) | claude |
| Global Store Consolidation | [docs/design/_archive/global-store-consolidation/](./_archive/global-store-consolidation/) | codex |
| Host Registry | [docs/design/host-registry/](./host-registry/) | claude |
| Knowledge graph | [docs/design/_archive/knowledge-graph/](./_archive/knowledge-graph/) | claude |
| MCP Bridge | [docs/design/mcp-bridge/](./mcp-bridge/) | codex |
| MCP Session Context | [docs/design/mcp-session-context/](./mcp-session-context/) | codex |
| Orbit Core | [docs/design/orbit-core/](./orbit-core/) | claude |
| Orbit Docs | [docs/design/orbit-docs/](./orbit-docs/) | claude |
| Orbit Graph | [docs/design/_archive/orbit-graph/](./_archive/orbit-graph/) | claude |
| Orbit Search | [docs/design/orbit-search/](./orbit-search/) | claude |
| Policy & Sandboxing | [docs/design/policy-sandbox/](./policy-sandbox/) | claude |
| Project Learnings | [docs/design/project-learnings/](./project-learnings/) | claude |
| Remote Access | [docs/design/remote-access/](./remote-access/) | claude |
| Resident Orchestrator | [docs/design/resident-orchestrator/](./resident-orchestrator/) | codex |
| Routines | [docs/design/routines/](./routines/) | claude |
| Task Artifacts | [docs/design/task-artifacts/](./task-artifacts/) | codex |
| Task Sync (archived) | [docs/design/_archive/task-sync/](./_archive/task-sync/) | claude |
| User Interface | [docs/design/user-interface/](./user-interface/) | gemini |
| Worktree Artifacts | [docs/design/worktree-artifacts/](./worktree-artifacts/) | codex |

Ownership means: the lead is accountable for keeping the folder's docs in sync with implementation, for flipping ADR status when tasks ship, and for responding to cross-review comments. Ownership does not preclude other agents from editing — it names who's on the hook when things drift.

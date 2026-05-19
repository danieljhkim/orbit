---
title: Design Doc Conventions
owner: daniel
last_updated: 2026-05-18
status: Accepted
---

# Design Doc Conventions

Rules feature leads follow when writing and maintaining design docs under `docs/design/<feature>/`. The goal is a set of feature folders that read as one coherent documentation system regardless of which agent authored them.

This doc is itself the source of truth for the conventions. When a convention changes, update this doc and then update existing feature folders to match — do not silently diverge.

---

## 1. Folder Layout (per feature)

```
docs/design/<feature>/
├── 1_overview.md       required — what and why
├── 2_design.md         required — current implementation
├── 3_vision.md         required — forward-looking
├── 4_decisions.md      required — ADR log (append-only)
├── specs/              required folder; may be empty initially
│   └── <mechanism>.md  one mechanism per file
└── references/         required folder; may be empty initially
    └── glossary.md     recommended; other lookup-style docs allowed
```

- Folder name: lowercase, hyphenated, singular (`knowledge-graph`, `groundhog`).
- No `README.md`, `roadmap.md`, `changelog.md`, `tutorial.md` at this level.
- No top-level narrative files outside the numbered four (`1_`–`4_`).

---

## 2. Required Frontmatter (all numbered docs)

Every numbered design doc starts with YAML frontmatter:

```yaml
---
title: <Feature> — <Doc Role>
owner: <agent family — codex, claude, grok, or gemini>
last_updated: YYYY-MM-DD
status: <Draft | Accepted>
feature: <feature-slug — matches the folder name>
doc_role: <overview | design | vision | decisions>
---

# <Feature> — <Doc Role>
```

- `title` mirrors the H1 verbatim.
- `owner` is the accountable agent family, not a committer list or full model string.
- `last_updated` is the calendar date of the last meaningful content change. Trivial reformat commits should not reset it.
- `status` is `Draft` until the doc is approved by the feature lead, then `Accepted`. It moves back to `Draft` if a structural rewrite is in flight.
- `feature` is the folder slug (e.g. `groundhog`, `knowledge-graph`). Lets tooling group docs by feature without parsing paths.
- `doc_role` is one of `overview`, `design`, `vision`, `decisions` — corresponds 1:1 with the filename prefix `1_`/`2_`/`3_`/`4_`.

Orbit-docs also indexes any configured Markdown doc with this retrieval frontmatter. For numbered design docs, keep the fields above and add at least `type` and `summary`:

```yaml
---
type: design | pattern | context | glossary | runbook
summary: One-line hook for agent retrieval
tags: [feature-slug]
paths: ["crates/orbit-cli/**"]
related_features: [feature-slug]
related_artifacts: [ORB-00160, ADR-0168, L20260514-3]
---
```

`type` and `summary` are required by the strict parser. `summary` must be a non-empty single line. `related_artifacts` accepts `ORB-NNNNN`, `LYYYYMMDD-N`, `FYYYY-MM-NNN`, and `ADR-NNNN` strings. The tolerant indexer infers these fields for legacy design docs and pattern docs, but new docs should write them explicitly. Orbit-docs does not index `.orbit/`; ADR bodies remain owned by `orbit-adr`.

---

## 3. Required Sections per Numbered Doc

| File | Required sections (in order) |
|------|------------------------------|
| **1_overview.md** | Elevator paragraph · §1 Motivation · §2 Core Concepts · §3 At a Glance (table: concern → file → task) · Task References |
| **2_design.md** | Scope paragraph · mechanism sections (variable count, numbered) · §N Concerns & Honest Limitations (mandatory last section) · Task References |
| **3_vision.md** | Scope paragraph · §1 Open Questions (numbered) · §2 Prior Work (subsections by category) · §3 What May Be Distinctive · §4 References (Orbit-internal + External) · Task References |
| **4_decisions.md** | Format explainer · ADR entries in ascending number order |

Every numbered doc ends with a **Task References** section listing only the task IDs cited in that doc, plus the line:

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---

## 4. ADR Template (strict)

**Allocation order is non-negotiable.** Before writing the heading or body, allocate the global ID via `orbit.adr.add` (see the `orbit-adr` skill and [ADR-0153]). The local heading then uses the allocated global ID verbatim — never invent a four-digit number that "looks global." The store is the source of truth for ID, status, owner, `related_features`, and `related_tasks`; the local `4_decisions.md` entry is the long-form narrative log keyed on that same global ID.

```
## ADR-NNNN — <short title, noun phrase>

**Status:** <Accepted | Proposed | Superseded by ADR-MMMM> · YYYY-MM · [T...]

**Context.** <1–3 sentences. Why this forced a decision.>

**Decision.** <1–3 sentences. What we chose.>

**Consequences.**
- <bullet>
- <bullet>
- Cost: <explicit tradeoff — every ADR must name at least one cost>
```

Rules:

- **Allocate first.** `orbit.adr.add` returns the global `ADR-NNNN` (4-digit, zero-padded). That ID is your local heading. Never hand-author headings under `## ADR-` without an allocation behind them. Bypassing this is the failure mode [ORB-00098] resolved; see [ADR-0153].
- **Inline cross-references** use the global ID (`[ADR-0042]`), resolvable via `orbit tool run orbit.adr.show --input '{"id":"ADR-0042"}'`.
- Numbers are append-only; superseded entries stay in place with status updated.
- `Proposed` allowed only before the relevant task ships. Flip to `Accepted` + task ID via `orbit.adr.update` (which also flips the markdown status line on the next edit) when it lands.
- Every ADR must cite at least one cost. No cost = the decision wasn't real.
- **Legacy 3-digit headings.** Existing local 3-digit headings (`## ADR-NNN`) authored before the global-ID convention are grandfathered. They may be backfilled opportunistically when a folder is being substantially edited; when backfilled, the local heading is rewritten to the allocated global ID and the original local ID is preserved as a `legacy_id` in the store record. See `docs/design/project-learnings/4_decisions.md` and `docs/design/agent-families/4_decisions.md` for worked examples.

An entry earns its own ADR only if **all three** hold:

1. **Real alternative.** A different choice was on the table and would have produced a materially different design — not "we did the obvious next instance of an existing pattern."
2. **Forward constraint.** The decision shapes future work, rules out a class of approaches, or imposes a non-trivial tradeoff readers will need to know about months later.
3. **Non-trivial cost.** The cost line names something a reader couldn't infer from the decision itself ("we now depend on grammar X" is trivial; "stable_id reallocates every object hash on first rebuild" is not).

If only one or two hold, the decision belongs in `2_design.md` prose, as a row in an existing ADR's table, or — for plain-instance work — as a task-ID citation on the parent ADR's Status line.

---

## 4a. Rollup ADRs

When a cluster of accepted ADRs all instantiate the same underlying decision (e.g. "added language X to the tree-sitter extractor set"), the cluster may be folded into a single rollup ADR:

- The rollup either reuses the parent ADR's number with an expanded body and a per-instance table, or claims a new number that lists the cluster.
- Each folded entry stays in place with `Status: Superseded by ADR-NNN (folded)` and a one-line pointer; the body is removed.
- The rollup must preserve every Cost line from the folded entries that doesn't duplicate a cost already named.
- Compaction is a normal maintenance operation, not an emergency cleanup. Owners should fold a cluster when the third instance lands, not the tenth.

---

## 5. Glossary Format

```
# Glossary: <Feature>

<One paragraph: what's in scope, what's deliberately excluded (generic terms).>

| Term | Meaning |
|------|---------|
| **Term** | Definition with cross-ref to [2_design.md §X]. |
```

Rules:

- Alphabetized.
- Orbit-specific vocabulary only. Standard industry terms (hunk, blob, content-addressed, TTL) are excluded by default unless the feature gives them a specific meaning.
- Every entry references the doc where the term is used, so definitions don't drift from implementation.

---

## 6. Spec Format (`specs/<mechanism>.md`)

```
# Spec: <Mechanism>

<One-paragraph contract statement.>

## Why This Exists
## <Mechanism-specific sections>
## Agent Signature (optional — who last revised)
```

A spec is **prescriptive**. It names invariants ("writes do not fall back"), failure modes ("detached HEAD errors"), and migration paths. It is *not* a design-rationale doc — rationale lives in `4_decisions.md`.

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

Two mechanical checks worth adding later:

1. Lint: every numbered doc has required frontmatter + Task References section.
2. Lint: every ADR has a Cost line.

Until those exist: cross-review is the enforcement mechanism. When one agent reviews the other's docs (KG ↔ Groundhog), the reviewer treats this doc as a checklist and rejects on any violation.

---

## 12. Ownership

| Feature | Folder | Lead |
|---------|--------|------|
| Activity / Job | [docs/design/activity-job/](./activity-job/) | codex |
| Auditability | [docs/design/auditability/](./auditability/) | codex |
| Groundhog | [docs/design/groundhog/](./groundhog/) | codex |
| Knowledge graph | [docs/design/knowledge-graph/](./knowledge-graph/) | claude |
| Policy & Sandboxing | [docs/design/policy-sandbox/](./policy-sandbox/) | claude |
| Project Learnings | [docs/design/project-learnings/](./project-learnings/) | claude |
| Task Artifacts | [docs/design/task-artifacts/](./task-artifacts/) | codex |
| Task Lineage | [docs/design/task-lineage/](./task-lineage/) | claude |
| Task Sync | [docs/design/task-sync/](./task-sync/) | claude |
| User Interface | [docs/design/user-interface/](./user-interface/) | gemini |

Ownership means: the lead is accountable for keeping the folder's docs in sync with implementation, for flipping ADR status when tasks ship, and for responding to cross-review comments. Ownership does not preclude other agents from editing — it names who's on the hook when things drift.

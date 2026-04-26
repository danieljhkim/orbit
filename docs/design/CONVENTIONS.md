# Design Doc Conventions

**Status:** Accepted
**Owner:** daniel
**Last updated:** 2026-04-26

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

```
# <Feature> — <Doc Role>

**Status:** <Draft | Accepted>
**Owner:** <agent identity — `claude`, `codex`, etc.>
**Last updated:** YYYY-MM-DD
```

Owner field is mandatory. It's the accountable agent, not a committer list.

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

```
## ADR-NNN — <short title, noun phrase>

**Status:** <Accepted | Proposed | Superseded by ADR-MMM> · YYYY-MM · [T...]

**Context.** <1–3 sentences. Why this forced a decision.>

**Decision.** <1–3 sentences. What we chose.>

**Consequences.**
- <bullet>
- <bullet>
- Cost: <explicit tradeoff — every ADR must name at least one cost>
```

Rules:

- Numbers are append-only; superseded entries stay in place with status updated.
- `Proposed` allowed only before the relevant task ships. Flip to `Accepted` + task ID when it lands.
- Every ADR must cite at least one cost. No cost = the decision wasn't real.

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
- Never link a task ID — `[T20260421-0528]` stays as plain bracketed text. It's searchable via `git log --grep=` regardless of where tasks are stored.
- Section references use full paths: `[2_design.md §6.3]`, not a bare `§6.3` from a sibling doc.

---

## 9. Task ID Citation Format

- Inline: plain bracketed text `[T20260421-0528]`.
- In ADRs: on the status line after the date.
- Never cite a task without naming what that task did — `([T20260421-0528])` alone is opaque; always give a verb phrase.

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
| Knowledge graph | [docs/design/knowledge-graph/](./knowledge-graph/) | claude |
| Groundhog | [docs/design/groundhog/](./groundhog/) | codex |
| Policy & Sandboxing | [docs/design/policy-sandbox/](./policy-sandbox/) | claude |

Ownership means: the lead is accountable for keeping the folder's docs in sync with implementation, for flipping ADR status when tasks ship, and for responding to cross-review comments. Ownership does not preclude other agents from editing — it names who's on the hook when things drift.

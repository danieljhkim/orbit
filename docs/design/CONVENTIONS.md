---
title: Design Doc Conventions
owner: daniel
last_updated: 2026-08-10
last_validated: 2026-08-10
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
├── 4_decisions.md      recommended — ADR entries (the record, not a pointer index)
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

The template frontmatter also carries the orbit-docs retrieval fields (`type`, `summary`, `tags`, `paths`, `related_features`, `related_artifacts`) so the doc is indexable on day one. `type` and `summary` are required by the strict parser. `summary` must be a non-empty single line. `related_artifacts` accepts `ORB-NNNNN`, `L-NNNN`, `FYYYY-MM-NNN`, and `ADR-NNNN` strings. The tolerant indexer infers these fields for legacy design docs and pattern docs, but new docs should write them explicitly. ADR bodies live in their feature's `4_decisions.md` and are indexed with every other design doc (§4) — the docs indexer still does not index `.orbit/`, which no longer holds anything authoritative.

---

## 3. Required Sections per Numbered Doc

| File | Required sections (in order) |
|------|------------------------------|
| **1_overview.md** | Elevator paragraph · §1 Motivation · §2 Core Concepts · §3 At a Glance (table: concern → file → task) · Task References |
| **2_design.md** | Scope paragraph · mechanism sections (variable count, numbered) · §N Concerns & Honest Limitations (mandatory last section) · Task References |
| **3_vision.md** | Scope paragraph · §1 Open Questions (numbered) · §2 Prior Work (subsections by category) · §3 What May Be Distinctive · §4 References (Orbit-internal + External) · Task References |
| **4_decisions.md** | Scope explainer · ADR entries in ascending number order, each with Status · Context · Decision · Consequences (incl. `Cost:`) |

Every numbered doc ends with a **Task References** section listing only the task IDs cited in that doc, plus the line:

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---

## 4. ADRs (strict)

**`4_decisions.md` is the record.** An ADR is a section in its feature's `4_decisions.md` — git-committed markdown, reviewed in the same PR as the code it describes, indexed by the ordinary docs corpus. There is no ADR store, no allocator, no lifecycle tool, and no `.orbit/adrs/` partition; that surface was retired for the reasons in §4b. `4_decisions.md` is no longer a pointer index into a second system of record.

### 4a. What earns an ADR

An ADR is admitted through exactly one of two doors. Most decisions go through neither.

**Door 1 — it explains surprising code.** A future reader will hit a specific site, think *this looks wrong*, and be right to think so until they know the decision. The test is concrete: name the file, ideally the function.

**Door 2 — it governs future decisions.** A standing preference or constraint that decides tradeoffs the project has not yet encountered — "prefer coverage over precision when the two conflict," "fail closed rather than degrade silently." These cannot be anchored to a site because they apply everywhere, and they are load-bearing precisely because they are invoked repeatedly across unrelated work.

Door 2 has one discipline, and the convention collapses without it: **the entry must be applicable to a case nobody has seen yet.** Write it as a rule a reader could apply to tomorrow's decision. A retrospective account of a choice already made — "we chose a native primitive over a flat markdown directory" — reads like Door 2 and is not: it settles one past question and governs nothing. That belongs in `2_design.md` prose.

Whichever door, the entry must still name a **real alternative** (a different choice was on the table and would have produced a materially different design) and a **non-trivial cost** (something a reader could not infer from the decision itself). No cost line means the decision wasn't real.

Everything else — organizational choices, crate boundaries, the obvious next instance of an existing pattern — is design prose. Put it in `2_design.md`, or cite the task ID on an existing ADR's Status line.

### 4b. Why the store was retired

Measured over the 210 accepted-and-superseded records the store held: 39% were cited from code, 55% only from other design docs, and 24% from nowhere at all. The split was qualitatively clean — code-cited entries described runtime behaviour contracts; doc-only entries described how the tree was arranged. Against that, the store cost ten CLI subcommands, an MCP write surface whose supersede path silently half-worked, a `proposed/` partition that the workspace-init gitignore template hid inside run worktrees — requiring a host-side staging handoff just to ship a draft — a dashboard API, a search index redundant with the docs corpus, and a dormant hub-global sequence allocator. Two systems of record for one decision, and the expensive one was not the one being read.

Migration was mechanical rather than a rescue: all 231 tracked store bodies moved verbatim into their feature's `4_decisions.md` before the store and its tool surface were retired by [ORB-10726].

Retiring it keeps every decision and drops the bookkeeping. Door 2 was added after the first draft of this rule made code-citation the sole test, which would have discarded the project's standing preferences along with the noise.

### 4c. Format and numbering

Copy the entry skeleton from [`_templates/4_decisions.md`](./_templates/4_decisions.md). Each entry is a `## ADR-NNNN — <title>` heading followed by **Status**, **Context**, **Decision**, and **Consequences**, the last carrying at least one `Cost:` line.

- **Numbering is repo-local.** Take the next unused four-digit number in this repo — `grep -rho 'ADR-[0-9]\{4\}' docs/ | sort -u | tail -1`. Numbers are append-only and never reused.
- **Never cite an ADR across repos.** `ADR-0234` means one thing in this repo and something else in another. Cross-repo references name the repo and the decision in prose.
- **Door 1 entries carry `code_anchors:`** — a list of paths, ideally `path::symbol` — and each anchored site carries a `// ADR-NNNN` comment pointing back. Both directions or neither: an ADR nobody can stumble into from the code cannot do the job it was admitted for, and a comment pointing at nothing is worse than no comment.
- **Door 2 entries carry `scope:`** instead — the areas the rule governs. These are the entries worth surfacing to an agent up front, since their whole value is being consulted before a decision rather than after a surprise.
- **Supersession is a status line**, not a lifecycle operation: `**Status:** Superseded by ADR-NNNN · <date>`. The superseded entry stays where it is with its body intact, so the reason the old architecture existed is not rewritten after the fact. A stale `// ADR-NNNN` anchor pointing at a superseded decision is actively misleading — see the lint in §11.
- **`Proposed` is allowed** only while the relevant task is in flight, and only in the feature branch. Nothing merges to the default branch still marked `Proposed`.
- **Legacy 3-digit headings** (`## ADR-NNN`) predate four-digit numbering and are grandfathered as-is. Renumbering them would break existing citations for no gain.

### 4d. Agents do not mint ADRs

An executing agent files a friction or raises the question in its run summary; it never adds an ADR entry. Authoring is deliberate and human-or-orchestrator driven. This is the rule that keeps the corpus small — the retired store's noise came overwhelmingly from decisions minted mid-run, when the author had the least context about whether the choice was novel.

### 4e. Rollup ADRs

When a cluster of accepted ADRs all instantiate the same underlying decision (e.g. "added language X to the tree-sitter extractor set"), fold the cluster into a single rollup entry:

- The rollup either reuses the parent's number with an expanded body and a per-instance table, or claims a new number that lists the cluster.
- Each folded entry keeps its heading and gets `**Status:** Superseded by ADR-NNNN (folded)`.
- The rollup must preserve every Cost line from the folded entries that doesn't duplicate a cost already named.
- Compaction is normal maintenance, not emergency cleanup. Fold when the third instance lands, not the tenth.

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

A spec is **prescriptive**. It names invariants ("writes do not fall back"), failure modes ("detached HEAD errors"), and migration paths. It is *not* a design-rationale doc — rationale lives in feature `4_decisions.md` entries.

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

Five mechanical checks worth adding later (as optional lints, never blocking):

1. Lint: every numbered doc has required frontmatter + Task References section.
2. Lint: every ADR has a Cost line.
3. Lint: every ADR is admitted through exactly one door — it carries `code_anchors:` or `scope:`, not both and not neither (§4c).
4. Lint: every path in a `code_anchors:` list exists and carries a matching `// ADR-NNNN` comment, and every `// ADR-NNNN` in the tree resolves to an entry. This is the check that keeps Door 1 honest in both directions.
5. Lint: no code cites a superseded ADR. A stale anchor is worse than no anchor, and this is the one failure mode retiring the lifecycle tooling makes more likely rather than less.

Check 4 also gives a maintenance signal worth acting on: a Door 1 entry with zero live citations has stopped explaining anything and is a demotion candidate.

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
| MCP Bridge | [docs/design/mcp-bridge/](./mcp-bridge/) | claude |
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
| Terminal Interface | [docs/design/terminal-interface/](./terminal-interface/) | claude |
| User Interface | [docs/design/user-interface/](./user-interface/) | gemini |
| Worktree Artifacts | [docs/design/worktree-artifacts/](./worktree-artifacts/) | codex |

Ownership means: the lead is accountable for keeping the folder's docs in sync with implementation, for flipping ADR status when tasks ship, and for responding to cross-review comments. Ownership does not preclude other agents from editing — it names who's on the hook when things drift.

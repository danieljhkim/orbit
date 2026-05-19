---
summary: "ADR Artifact — Overview"
type: design
title: "ADR Artifact — Overview"
owner: claude
last_updated: 2026-05-10
status: Draft
feature: adr-artifact
doc_role: overview
tags: ["adr-artifact"]
---

# ADR Artifact — Overview

The ADR artifact proposal lifts Architecture Decision Records out of per-feature `4_decisions.md` markdown files into a first-class Orbit artifact — alongside Tasks, Activities, Jobs, and Skills. Each ADR gets a stable globally-unique ID, structured lifecycle state, owner, supersession edges, and is queryable through an `orbit.adr.*` tool surface. Feature design docs reference decisions by ID (`[ADR-042]`) instead of inlining them, and `2_design.md` returns to present-tense current-state writing. **This is a v2 proposal.** v1 ships unchanged; v2 migrates the existing markdown ADR corpus into the store and retires `4_decisions.md` as a hand-maintained file.

This document is the entry point. [2_design.md](./2_design.md) specifies the schema, scoping, tool surface, and migration mechanics; [3_vision.md](./3_vision.md) names open questions; [4_decisions.md](./4_decisions.md) is the ADR log for this feature (in markdown until the artifact it describes exists).

---

## 1. Motivation

The current ADR layout — `docs/design/<feature>/4_decisions.md` as an append-only markdown log — was right for the first dozen ADRs and is getting wrong as the feature folders mature. Three concrete problems, ranked by visibility:

1. **Unbounded markdown growth.** ADRs are append-only by convention and never get removed. `activity-job/4_decisions.md` already contains dozens of entries spanning a year of work; reading it linearly to find a single decision is slow, and there is no index, filter, or search beyond `grep`.

2. **Cross-feature decisions live in one folder by accident.** A decision about audit-event redaction touches activity-job, auditability, and the executor. Today it lives in whichever folder its author opened first; other folders either don't reference it or duplicate the rationale. The folder structure pretends every decision is feature-local, but many aren't.

3. **No structured lifecycle.** `Status: Proposed | Accepted | Superseded by ADR-MMM` is a markdown line a human flips. There is no audit trail of the flip, no enforcement that `Accepted` carries a task ID, and no way to answer *"what ADRs are still proposed and unshipped?"* without grepping ten files.

A fourth problem is downstream of the first three: **`2_design.md` bloats with change-history narrative** (*"After [T...] we did X, then [T...] Y..."*) because the ADR log isn't a natural query target. If `2_design.md` could cite `[ADR-042]` and the reader could resolve it with one tool call, the narrative wouldn't need to inline the history.

Making ADRs a first-class artifact dissolves all four:

- A queryable store eliminates linear-scan reading.
- Cross-feature decisions are referenced from N feature docs through a single record, without duplication.
- Lifecycle becomes real state with audit trail and supersession edges.
- `2_design.md` returns to present-tense and cites `[ADR-NNN]` for *why* and *when*.

---

## 2. Core Concepts

### 2.1 The ADR artifact

A typed record with structured metadata and a markdown body:

- `id` — globally unique, monotonic (proposed: `ADR-NNNN`, zero-padded)
- `title` — short noun phrase
- `status` — `proposed | accepted | superseded`
- `owner` — agent identity (e.g. `claude`, `codex`)
- `context`, `decision`, `consequences` — markdown sections matching today's ADR template; at least one `consequences` bullet must be a labeled `Cost:` line, preserving the existing rule
- `supersedes` / `superseded_by` — optional ID references for chains
- `related_features` — list of feature folder names this decision touches (N:M, not 1:1)
- `related_tasks` — list of Orbit task IDs that proposed or shipped the decision
- `legacy_id` — optional, set during migration (e.g. `"activity-job/ADR-017"`) so historical references resolve

Numbering is **global, not per-feature.** The current per-feature scheme (`ADR-001` exists in multiple folders) makes cross-folder reference ambiguous and is the first thing migration breaks. Globally unique IDs let any doc cite any ADR without folder qualification.

### 2.2 Lifecycle and supersession

Three states, with real transitions:

- `proposed` — design exists, implementation hasn't shipped. Created by `orbit.adr.add`.
- `accepted` — implementation landed. Set by `orbit.adr.update --status=accepted --task=T...`. The task ID is required for this transition, matching the existing `Proposed → Accepted` rule.
- `superseded` — a later ADR replaces this one. Set by `orbit.adr.supersede --by=ADR-NNNN`, which writes bidirectional links on both records.

There is no `rejected` or `withdrawn` state. A proposed ADR that won't ship is deleted by the owner; an accepted ADR that the team backs out of is superseded by a new ADR explaining the reversal. This matches what `4_decisions.md` already does and avoids inventing a fourth state for the artifact to carry.

### 2.3 Reference from design docs and tool surface

Feature `2_design.md` files reference decisions by ID inline: *"Backend resolution follows [ADR-042]."* Lookup is one tool call (`orbit.adr.show ADR-042`) instead of opening a sibling markdown file and grepping for the heading.

`4_decisions.md` becomes either a thin generated per-feature index (`orbit.adr.list --feature=activity-job`) or is retired entirely — see [3_vision.md §1](./3_vision.md) for the open question.

The tool surface, matching the existing task/state patterns:

- `orbit.adr.add` — create a proposed ADR; returns assigned ID
- `orbit.adr.show <id>` — render full ADR with metadata
- `orbit.adr.list [--feature] [--status] [--owner]` — filtered index
- `orbit.adr.update <id>` — flip status, edit body, attach task refs
- `orbit.adr.supersede <old-id> --by <new-id>` — write bidirectional edges
- `orbit.adr.search <query>` — semantic search, reusing `orbit-embed::vector`

### 2.4 v1/v2 boundary and migration

v1 ships unchanged: `4_decisions.md` remains the source of truth, this folder is docs-only, and there is no `orbit.adr.*` code. v2 ships:

1. Schema and store (`orbit-common::types::adr`, `orbit-store::adr_store`).
2. Tool surface (`orbit-tools::adr_tool`) and CLI commands.
3. One-shot migration tool that walks every `docs/design/*/4_decisions.md`, parses each ADR entry against the existing template, and writes one artifact per entry with `legacy_id` populated.
4. CONVENTIONS.md update that drops the `4_decisions.md` requirement (or redefines it as generated).
5. Sweep of feature `2_design.md` files to replace inline `[T...]` change-history with `[ADR-NNNN]` citations where applicable — this is the leverage point for trimming wordiness.

The v1→v2 boundary is itself an ADR ([4_decisions.md ADR-001](./4_decisions.md)).

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Existing convention this proposal modifies | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Existing ADR corpus (migration sources) | `docs/design/*/4_decisions.md` (10 folders) | — |
| Store pattern to follow | [crates/orbit-store/src/file/task_store/](../../../crates/orbit-store/src/file/task_store/) | — |
| Scoping precedent (provisional `WorkspaceOnly`) | [crates/orbit-store/src/scope.rs](../../../crates/orbit-store/src/scope.rs) | — |
| Semantic-search reuse for `orbit.adr.search` | [crates/orbit-embed/src/vector/](../../../crates/orbit-embed/src/vector/) | — |
| Tool registry pattern for `orbit.adr.*` | [crates/orbit-tools/](../../../crates/orbit-tools/) | — |
| Schema, tool surface, migration mechanics | [2_design.md](./2_design.md) | — |
| Open questions (retire `4_decisions.md`, cross-workspace scoping) | [3_vision.md](./3_vision.md) | — |
| ADR log for this feature | [4_decisions.md](./4_decisions.md) | — |

---

## Task References

- [T20260510-27] — Drafted the adr-artifact design folder as a v2 proposal: motivation, schema, lifecycle, tool surface, migration, and the ADR log resolving the open questions raised during authoring.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

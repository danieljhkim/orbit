---
summary: "Project Learnings — Overview"
type: design
title: "Project Learnings — Overview"
owner: claude
last_updated: 2026-05-17
status: Draft
feature: project-learnings
doc_role: overview
tags: ["project-learnings"]
---

# Project Learnings — Overview

Project learnings is a system for preserving and surfacing non-obvious project knowledge — gotchas, root causes from incidents, validated approaches, hard-won workflow insights — at the moment of action so agents stop repeating the same mistakes. The system is **push-first**: relevant learnings inject into agent context automatically when an agent is about to touch code, files, or workflows the learning applies to. A pull surface exists for active exploration, but pull is the secondary mode.

Phase 1 ships the native primitive (`learning` resource type alongside `task`), the push-injection mechanism, and the pull skill. Phase 2, deferred until [docs/design/semantic-search/](../semantic-search/) reaches Accepted, layers semantic-similarity ranking on top of the path-glob scoping that phase 1 uses.

This document is the entry point. [2_design.md](./2_design.md) specifies the storage schema, injection pipeline, lifecycle, and surface; [3_vision.md](./3_vision.md) names open questions and prior art; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

Three concrete failure modes exist today, none of which the existing knowledge surfaces (`CLAUDE.md`, design ADRs, agent `MEMORY.md`, `/learn`) close:

1. **Repeated mistakes.** An agent declares a performance win on latency alone, gets corrected, and re-learns the lesson on the next benchmark task. The correction lives in agent-private memory (`~/.claude/.../memory/feedback_perf_correctness_audit.md`) or a commit message; the next agent — or the same agent in a fresh session on a different machine — doesn't see it. The kind of knowledge this system is meant to elevate from per-agent memory to project artifact.
2. **Postmortem decay.** Root causes from incidents land as commit messages and review-thread replies, then become unsearchable under their original framing. A future agent investigating the same area has no way to encounter the prior incident's lesson except by chance.
3. **Cross-cutting knowledge is homeless.** ADRs scope to a feature folder. CLAUDE.md is loaded on every session and gets noisy fast. Workspace-private MEMORY.md is per-agent and per-machine. None of these handle "when editing anything that touches both `orbit-store` and the activity-job runner, remember Y."

Pull-only systems (flat markdown directories, search tools, the `/learn` skill) require an agent to remember to look. **That is exactly the failure mode the system is built to prevent**: if discovery depends on agent discipline, the agent that needed the learning most — the one that forgot it existed — won't find it. Push-first delivery resolves this by surfacing the learning at the moment of action without requiring the agent to query for it.

The hard constraint that shapes the design: **the system must be discoverable across agents, not just Claude Code.** Orbit runs Codex, Gemini, Claude, and others through the activity/job runner. A solution that hooks only into Claude Code's `PreToolUse` would re-fragment knowledge along agent-vendor lines, which defeats the point.

---

## 2. Core Concepts

### 2.1 Learning record

A first-class Orbit resource, parallel to `task`. Each record carries:

- `id` — `L20260509-NNNN`, allocated like task IDs.
- `scope` — what triggers the learning. Phase 1: path globs + tags. Phase 2 will layer semantic similarity on top ([4_decisions.md ADR-004](./4_decisions.md)).
- `summary` — one-line rule of thumb (the part that fits in a `<system-reminder>`).
- `body` — multi-line markdown: the rule, the reason, how to apply it.
- `evidence` — commit SHAs, task IDs, or external refs that produced the learning.
- `status` — `active` or `superseded`.
- vote sidecar — append-only task-anchored re-validation events, stored outside the YAML.
- `supersedes` — back-reference when a newer learning replaces an older one.
- `created_by`, `created_at`, `updated_at` — provenance.

Records persist as YAML on disk under `.orbit/learnings/<id>/learning.yaml`, with sidecars such as `votes.jsonl` living beside the YAML. Workspace-scoped per the Scoping Rules table in [CLAUDE.md](../../../CLAUDE.md), and checked into git so learnings travel with the repo ([4_decisions.md ADR-003](./4_decisions.md)).

### 2.2 Push-based discovery

Learnings reach agents through three injection points, layered from coarsest to finest:

1. **Engine pre-prompt injection.** When `orbit-engine` spawns an agent for a task, it queries learnings whose scope matches the task's `context_files` and prepends matching summaries to the agent prompt. Universal across agents because it happens above the agent boundary ([2_design.md §4](./2_design.md)).
2. **MCP tool-call injection.** When an agent calls an Orbit MCP tool that references file paths (`orbit_graph_show`, `orbit_task_show`, etc.), the tool response carries a sidecar `learnings` field listing relevant entries. Works for any agent that speaks MCP.
3. **Claude Code `PreToolUse` hook.** Finer-grained per-edit injection on `Edit | Write | Read`. Covers Claude Code's built-in editor tools, which the MCP layer doesn't see. Optional: a layer of precision on top of (1) and (2), not a replacement.

Cap injection at 3–5 learnings per call and dedupe per session to keep context bounded ([4_decisions.md ADR-005](./4_decisions.md)).

### 2.3 Pull surface

For active exploration ("what should I know about this crate before I start?"), an `orbit-learnings` skill wraps `orbit.learning.search` with a natural-language interface. Agents can also call the tool directly. Pull is a complement to push, not the primary path; the push layer exists precisely because pull alone has been observed to fail.

### 2.4 Curation lifecycle

Active learnings can be superseded (replaced by a newer entry) or marked stale (the code they reference no longer exists). The knowledge graph is the natural staleness signal: when a symbol or file referenced in a learning's `evidence` disappears in a graph rebuild, the learning is flagged for review. Pruning is human-or-agent-driven; the system does not auto-delete.

### 2.5 Phase boundary

| Phase | Scope axis | Ranking | Discovery |
|-------|-----------|---------|-----------|
| **Phase 1** | path globs + tags | decay-weighted upvotes + manual priority + recency | engine pre-prompt + MCP injection + (optional) Claude Code hook |
| **Phase 2** | + symbol-aware (knowledge graph) | + semantic similarity (semantic-search) | + relevance-ranked, not just match-based |

Phase 2 is gated on [docs/design/semantic-search/](../semantic-search/) reaching Accepted because the relevance-ranking layer wants real semantic similarity, and the symbol-aware scope wants the same graph integration semantic-search phase 2 will require.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Architectural placement (storage in `orbit-store`, tools in `orbit-tools`) | [2_design.md §1](./2_design.md) | [T20260510-11] |
| Learning record schema | [2_design.md §2](./2_design.md) | [T20260510-11] |
| Scope axis (path globs + tags, phase 1) | [2_design.md §3](./2_design.md), [4_decisions.md ADR-004](./4_decisions.md) | [T20260510-11] |
| Push-injection pipeline | [2_design.md §4](./2_design.md), [4_decisions.md ADR-001](./4_decisions.md), [4_decisions.md ADR-005](./4_decisions.md) | [T20260510-11] |
| Prerequisite: `Task.tags` field | [2_design.md §4.1](./2_design.md) | [T20260510-12] |
| MCP / CLI surface (`orbit.learning.*`) | [2_design.md §5](./2_design.md) | [T20260510-11] |
| Re-validation votes and ranking | [2_design.md §5.4](./2_design.md), [4_decisions.md ADR-006](./4_decisions.md) | [ORB-00095] |
| Pull skill (`orbit-learnings`) | [2_design.md §6](./2_design.md) | [T20260510-11] |
| Curation lifecycle, supersession, staleness | [2_design.md §7](./2_design.md) | [T20260510-11] |
| Native primitive vs flat markdown | [4_decisions.md ADR-002](./4_decisions.md) | [T20260510-11] |
| Checked-in vs workspace-only state | [4_decisions.md ADR-003](./4_decisions.md) | [T20260510-11] |
| Concerns & honest limitations | [2_design.md §8](./2_design.md) | [T20260510-11] |
| Relationship to semantic-search | [3_vision.md §1.2](./3_vision.md), [docs/design/semantic-search/](../semantic-search/) | [T20260510-11] |
| Open questions, prior work | [3_vision.md](./3_vision.md) | [T20260510-11] |
| ADR log | [4_decisions.md](./4_decisions.md) | [T20260510-11] |

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [T20260510-12] — Add `tags` field to `Task` schema. Hard prerequisite for Layer 1's tag-axis matching.
- [ORB-00095] — Add task-anchored learning upvotes and decay-weighted search ranking.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

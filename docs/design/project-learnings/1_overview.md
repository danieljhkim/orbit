---
summary: "Project Learnings — Overview"
type: design
title: "Project Learnings — Overview"
owner: claude
last_updated: 2026-08-09
status: Draft
feature: project-learnings
doc_role: overview
tags: ["project-learnings"]
---

# Project Learnings — Overview

Project learnings is a system for preserving and surfacing non-obvious project knowledge — gotchas, root causes from incidents, validated approaches, hard-won workflow insights — so agents can retrieve and apply them when they are relevant. Delivery combines push and pull: engine pre-prompt injection and an MCP sidecar decorator push scope-matched learnings into every agent run automatically (source locations: [4_decisions.md ADR-0108 amendment](./4_decisions.md)), and agents can also pull with `orbit search` and open the full record with `orbit learning show`. Point-of-use reference comments make the relevant artifact discoverable in the code as a lighter-weight locator alongside both.

Phase 1 ships the native primitive (`learning` resource type alongside `task`), the pull surface, and the reference-comment convention. Phase 2, deferred until [docs/design/orbit-search/](../orbit-search/) reaches Accepted, layers semantic-similarity ranking on top of the path-glob scoping that phase 1 uses.

This document is the entry point. [2_design.md](./2_design.md) specifies the storage schema, pull-delivery mechanism, lifecycle, and surface; [3_vision.md](./3_vision.md) names open questions and prior art; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

Three concrete failure modes exist today, none of which the existing knowledge surfaces (`CLAUDE.md`, design ADRs, agent `MEMORY.md`, `/learn`) close:

1. **Repeated mistakes.** An agent declares a performance win on latency alone, gets corrected, and re-learns the lesson on the next benchmark task. The correction lives in agent-private memory (`~/.claude/.../memory/feedback_perf_correctness_audit.md`) or a commit message; the next agent — or the same agent in a fresh session on a different machine — doesn't see it. The kind of knowledge this system is meant to elevate from per-agent memory to project artifact.
2. **Postmortem decay.** Root causes from incidents land as commit messages and review-thread replies, then become unsearchable under their original framing. A future agent investigating the same area has no way to encounter the prior incident's lesson except by chance.
3. **Cross-cutting knowledge is homeless.** ADRs scope to a feature folder. CLAUDE.md is loaded on every session and gets noisy fast. Workspace-private MEMORY.md is per-agent and per-machine. None of these handle "when editing anything that touches both `orbit-store` and the activity-job runner, remember Y."

Unstructured pull-only systems (flat markdown directories and generic wikis) make a relevant record hard to find. Orbit instead makes the durable artifact searchable and puts a short reference comment at the affected code or workflow boundary. The comment names the artifact ID and why it applies; the agent can then retrieve the authoritative record rather than receive a broad, automatic reminder.

The hard constraint that shapes the design: **the system must be discoverable across agents, not just one agent vendor.** Orbit runs Codex, Gemini, Claude, and others through the activity/job runner. Search, show, and repository-local reference comments are vendor-neutral.

---

## 2. Core Concepts

### 2.1 Learning record

A first-class Orbit resource, parallel to `task`. Each record carries:

- `id` — `L-NNNN`, allocated like task IDs.
- `scope` — what triggers the learning. Phase 1: path globs + tags. Phase 2 will layer semantic similarity on top ([4_decisions.md ADR-004](./4_decisions.md)).
- `summary` — one-line rule of thumb displayed in a concise search result.
- `body` — multi-line markdown: the rule, the reason, how to apply it.
- `evidence` — commit SHAs, task IDs, or external refs that produced the learning.
- `status` — `active` or `superseded`.
- vote sidecar — append-only task-anchored re-validation events, stored outside the YAML.
- `supersedes` — back-reference when a newer learning replaces an older one.
- `created_by`, `created_at`, `updated_at` — provenance.

Records persist as YAML on disk under `.orbit/learnings/<id>/learning.yaml`, with sidecars such as `votes.jsonl` living beside the YAML. Workspace-scoped per the Scoping Rules table in [CLAUDE.md](../../../CLAUDE.md), and checked into git so learnings travel with the repo ([4_decisions.md ADR-003](./4_decisions.md)).

### 2.2 Pull-based discovery and reference comments

Agents discover a learning through `orbit search --kind learning` (or a topic/path-specific query) and read its authoritative body with `orbit learning show <id>`. `show` records the passive `learning_shown` usage signal.

When a learning or decision applies at a particular code or workflow boundary, add a concise nearby comment such as `// L-0041: hook subcommands keep parsing and state in core.` The comment is a locator, not a copy of the learning: it carries the artifact ID and a short rationale, while the full record remains in the Orbit registry. Do not place workspace-local artifact IDs in shipped prompts or other consumer-facing instruction assets.

### 2.3 Curation lifecycle

Active learnings can be superseded (replaced by a newer entry) or marked stale
when their path scope or cited task/commit evidence no longer resolves. Pruning
is human-or-agent-driven; the system does not auto-delete.

### 2.4 Phase boundary

| Phase | Scope axis | Ranking | Discovery |
|-------|-----------|---------|-----------|
| **Phase 1** | path globs + tags | manual priority + recency | `orbit search` / `orbit learning show` + reference comments |
| **Phase 2** | path globs + tags | + semantic similarity (orbit-search) | improved pull ranking |

Phase 2 is gated on [docs/design/orbit-search/](../orbit-search/) reaching
Accepted because the relevance-ranking layer wants real semantic similarity.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Architectural placement (storage in `orbit-store`, tools in `orbit-tools`) | [2_design.md §1](./2_design.md) | [T20260510-11] |
| Learning record schema | [2_design.md §2](./2_design.md) | [T20260510-11] |
| Scope axis (path globs + tags, phase 1) | [2_design.md §3](./2_design.md), [4_decisions.md ADR-004](./4_decisions.md) | [T20260510-11] |
| Pull delivery and reference comments | [2_design.md §4](./2_design.md) | [ORB-10346] |
| MCP / CLI surface (`orbit.learning.*`) | [2_design.md §5](./2_design.md) | [T20260510-11] |
| Re-validation votes and ranking | [2_design.md §5.4](./2_design.md), [4_decisions.md ADR-006](./4_decisions.md) | [ORB-00095] |
| Pull surface (`orbit search` / `orbit learning show`) | [2_design.md §6](./2_design.md) | [T20260510-11] |
| Curation lifecycle, supersession, staleness | [2_design.md §7](./2_design.md) | [T20260510-11] |
| Native primitive vs flat markdown | [4_decisions.md ADR-002](./4_decisions.md) | [T20260510-11] |
| Checked-in vs workspace-only state | [4_decisions.md ADR-003](./4_decisions.md) | [T20260510-11] |
| Concerns & honest limitations | [2_design.md §8](./2_design.md) | [T20260510-11] |
| Relationship to orbit-search | [3_vision.md §1.2](./3_vision.md), [docs/design/orbit-search/](../orbit-search/) | [T20260510-11] |
| Open questions, prior work | [3_vision.md](./3_vision.md) | [T20260510-11] |
| ADR log | [4_decisions.md](./4_decisions.md) | [T20260510-11] |

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [T20260510-12] — Add `tags` field to `Task` schema. Hard prerequisite for Layer 1's tag-axis matching.
- [ORB-00095] — Add task-anchored learning upvotes and decay-weighted search ranking.
- [ORB-10346] — Remove the Claude Code `PreToolUse` hook layer of automatic learning delivery; retain pull discovery, `learning_shown`, historical usage stats, and the still-active engine pre-prompt and MCP sidecar layers.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

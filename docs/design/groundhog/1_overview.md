---
summary: "Groundhog — Overview"
type: design
title: "Groundhog — Overview"
owner: codex
last_updated: 2026-05-12
status: Draft
feature: groundhog
doc_role: overview
tags: ["groundhog"]
---

# Groundhog — Overview

> *"The agent gets to retry each checkpoint like Bill Murray in Groundhog Day — it wakes up fresh but remembers what it learned."*

Groundhog is Orbit's checkpoint-oriented execution mode for HTTP-backed coding agents. It takes a structured task plan, runs one checkpoint at a time, and gives each attempt a fresh agent session plus a git-backed workspace snapshot. The intended payoff is smaller prompt state, cleaner retries, and higher confidence that "success" means the workspace really satisfies the requested checkpoint.

> **v1 release scope.** Groundhog is **not** part of the v1 release surface. v1 ships `backend: cli` as the only supported agent invocation path; Groundhog requires the HTTP `LoopTransport` and is therefore preview-only in v1. This document continues to describe Groundhog for design continuity — expect it to land as a supported surface in a v2 release once HTTP coverage is complete.

Today Groundhog exists as a partial but load-bearing implementation across `orbit-common`, `orbit-engine`, and `orbit-tools`. Read this overview first, then [2_design.md](./2_design.md) for the current contract and gap ledger, [3_vision.md](./3_vision.md) for open questions, [4_decisions.md](./4_decisions.md) for ADRs, and the focused `specs/` and `references/` files only when you need subsystem detail.

---

## 1. Motivation

Unstructured agent loops fail in predictable ways:

1. They carry too much scratch history forward. A failed attempt leaves behind prompt noise even when the useful lesson is small.
2. They retry from dirty workspace state. Partial edits survive failure and make the next attempt harder to reason about.
3. They blur "worked on it" with "finished it." Without an explicit success boundary, an agent can claim success before the workspace satisfies mechanical checks.

Groundhog narrows that surface. The plan defines checkpoint-sized subgoals, the runtime gives each attempt a clean workspace snapshot, and success is closed explicitly at a verification boundary.

---

## 2. Core Concepts

### 2.1 Structured checkpoint plans

Groundhog reads checkpoints from the task's structured `plan` field. Each checkpoint carries `id`, `spec`, typed `success_criteria`, and `attempt_budget`. That plan schema landed in [T20260420-0509-2].

### 2.2 Attempt-scoped execution

The Groundhog runner executes one checkpoint at a time. Each attempt gets a fresh agent session, Groundhog-specific builtins, the full task plan, summaries of prior successful checkpoints, the current checkpoint, and the latest failure report for the active checkpoint. The dedicated runner landed in [T20260420-0510-2]; the builtin tool surface landed in [T20260420-0509-3].

### 2.3 Git-backed rewind

Before an attempt starts, Groundhog snapshots the task branch into a scratch branch. Failure rewinds the task branch to the snapshot point; success squash-merges the scratch branch back as one checkpoint commit. That helper landed in [T20260420-0509-4].

### 2.4 Chronicle memory

Groundhog's persisted memory is currently a `Chronicle` of checkpoint records plus a separate runner-state artifact. The append-only serializer for that chronicle landed in [T20260420-0509]. The intended direction is "success-only prompt memory, richer audit off to the side"; [2_design.md](./2_design.md) names where the current implementation still differs.

### 2.5 Verification boundary

Mechanical success criteria belong to the runtime, not the agent's self-report. The shared checkpoint verifier landed in [T20260420-0510], and the Groundhog runner wires a checkpoint-success verification step into the attempt lifecycle in [T20260420-0510-2].

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Structured checkpoint plan parsing | `crates/orbit-common/src/types/task_plan.rs` | [T20260420-0509-2] |
| Groundhog activity runner | `crates/orbit-engine/src/activity_job/groundhog/mod.rs` plus sibling submodules | [T20260420-0510-2], [T20260509-19] |
| Git-backed snapshots and rewind | `crates/orbit-engine/src/workspace_snapshot.rs` | [T20260420-0509-4] |
| Chronicle types and serializer helpers | `crates/orbit-common/src/groundhog.rs` | [T20260420-0509] |
| Groundhog builtin verbs | `crates/orbit-tools/src/builtin/orbit/groundhog_*` | [T20260420-0509-3] |
| Shared checkpoint verifier | `crates/orbit-engine/src/checkpoint_verifier.rs` | [T20260420-0510] |

---

## Task References

- **[T20260420-0509]** — Add Groundhog chronicle serializer and shared Groundhog data types.
- **[T20260420-0509-2]** — Add structured task plan parsing with typed checkpoints and success criteria.
- **[T20260420-0509-3]** — Add Groundhog builtin verb tools.
- **[T20260420-0509-4]** — Add Groundhog workspace snapshots and scratch-branch rewind mechanics.
- **[T20260420-0510]** — Add the shared runtime checkpoint verifier.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260430-21]** — Shorten Groundhog design docs and remove obsolete top-level duplicates.
- **[T20260509-19]** — Split the Groundhog activity runner into focused engine submodules.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

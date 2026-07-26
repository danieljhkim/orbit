---
summary: "Activity / Job — Overview"
type: design
title: "Activity / Job — Overview"
owner: codex
last_updated: 2026-07-20
status: Draft
feature: activity-job
doc_role: overview
tags: ["activity-job"]
---

# Activity / Job — Overview

Activity / Job is Orbit's execution substrate. Activities describe runnable units; jobs compose them sequentially, in parallel, across collections, or through bounded loops. Orbit's product story is moving toward goals, graphs, sessions, and locks, but this layer remains the runtime underneath. [2_design.md](./2_design.md) is the current contract; [3_vision.md](./3_vision.md) captures open questions.

> **v1 release scope.** v1 ships `backend: cli` as the supported agent invocation path. HTTP `LoopTransport` (`backend: http`) exists in code and tests, but remains preview-only until v2.

---

## 1. Motivation

Orbit needs a runtime layer that humans can inspect and code can execute. Activity / Job solves four practical problems:

1. **Typed execution.** Agent loops and deterministic actions share one schema family.
2. **Durable local control flow.** Retry, parallelism, fan-out, and loops survive outside one model turn via `JobV2` DAG constructs.
3. **Clean runtime boundaries.** orbit-core coordinates runs without naming `orbit-agent` internals through the `V2RuntimeHost` work.
4. **One canonical schema.** `schemaVersion: 1` assets fail load-time parsing.

---

## 2. Core Concepts

### 2.1 Activities are the runnable units

An `ActivityV2` carries shared metadata plus one runtime spec:

- `agent_loop`
- `deterministic`

The shared shape shipped. The `shell` type was removed as a fail-closed security fix in [ORB-00374]; see [ADR-0194](./4_decisions.md). The `groundhog` activity kind was later removed as unused in [ORB-10332].

### 2.2 Jobs are the orchestration grammar

A `JobV2` is a step tree with:

- `when`
- `retry`
- flat target steps
- `target: activity:<name>` references
- `parallel`
- `fan_out` / `fan_in`
- `loop`

That grammar landed first, followed by the `workflow` / `subroutine` job kinds.

### 2.3 Load-time normalization is part of the contract

orbit-core normalizes assets before a run starts:

- loads YAML through a two-pass schema loader
- resolves `target: activity:<name>` references for jobs
- rewrites `backend: auto` to a concrete backend once per run
- rejects loop/session/backend combinations that cannot execute safely

Name resolution arrived first. Backend resolution and `run-v2` entrypoints came next; CLI backend support followed.

### 2.4 Backends and providers are separate choices

For `agent_loop`, Orbit distinguishes:

- **backend**: `http`, `cli`, or `auto`
- **provider**: `claude`, `codex`, `gemini`, `ollama`, or `openai_compat`

`backend: auto` resolves once at load time. `backend: http` against an unwired provider fails structurally instead of falling back. `backend: cli` intentionally retains the older CLI-provider runtimes.

### 2.5 Audit, policy, and seeded assets make the runtime inspectable

This layer also owns:

- `fsProfile` attachment on activities and target steps
- the v2 audit envelope with `workspace_path` provenance
- seeded reference assets and pipeline jobs used by `orbit init`

The envelope gained `workspace_path`; runtime/CLI `fsProfile` enforcement and init seeding followed.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| v2 activity type system | `crates/orbit-common/src/types/activity_job/activity_v2.rs` |  |
| v2 job step grammar | `crates/orbit-common/src/types/activity_job/job_v2.rs` |  |
| Job kinds (`workflow`, `subroutine`) | `crates/orbit-common/src/types/activity_job/job_v2.rs` |  |
| Target-ref resolution | `crates/orbit-common/src/types/activity_job/catalog.rs` |  |
| `run-v2` core entrypoints and host boundary | `crates/orbit-cmd/src/activity_v2.rs`, `crates/orbit-core/src/command/job/exec.rs` | |
| Backend resolution and loop/session constraints | `crates/orbit-core/src/command/backend_resolver.rs`, `crates/orbit-common/src/types/activity_job/backend.rs` |  |
| v2 DAG executor | `crates/orbit-engine/src/activity_job/job_executor/` | [T20260509-2] |
| V2 audit envelope and disk sink | `crates/orbit-common/src/types/activity_job/audit_envelope.rs`, `crates/orbit-engine/src/activity_job/audit_writer.rs` |  |
| `backend: cli` runtime path | `crates/orbit-engine/src/activity_job/cli_runner/mod.rs` |  |
| `fsProfile` enforcement | `crates/orbit-policy`, `tool_context_for_activity`, CLI describe/get surfaces |  |
| Seeded reference activities and pipeline jobs | `crates/orbit-core/assets/activities/`, `crates/orbit-core/assets/jobs/` | |

---

## Task References

- Add the first v2 activity runtime scaffolding.
- Add `JobV2` DAG constructs (`parallel`, `fan_out`, `loop`, `retry`, `when`).
- Add v2 activity name resolution and pipeline skeleton assets.
- Wire `V2RuntimeHost` in orbit-core and add `orbit activity run-v2`.
- Reshape `V2RuntimeHost` to keep `orbit-agent` types out of orbit-core.
- Add `workspace_path` provenance to the v2 audit envelope.
- Add `backend: cli` dispatch for v2 `agent_loop`.
- Add v2 job kinds to the job catalog.
- Enforce `fsProfile` rules across runtime and CLI surfaces.
- Add `task_gate_pipeline`.
- Add `task_auto_pipeline`.
- Retire v1 assets and drop the transitional v2 naming.
- Seed activities and workflows on `orbit init`.
- **[ORB-10332]** — Remove the unused Groundhog activity kind and the epic/parallel pipeline layer.
- **[T20260430-19]** — Shorten the Activity / Job design docs while preserving required structure.
- **[T20260509-2]** — Split the v2 job executor into responsibility-focused modules without changing runtime behavior.
- **[ORB-00374]** — Remove the `shell` activity variant and `run_shell` dispatch (fail-closed resolution of security bug [ORB-00363]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

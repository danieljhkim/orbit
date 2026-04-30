# Activity / Job — Overview

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-30

Activity / Job is Orbit's execution substrate. Activities describe runnable units; jobs compose them sequentially, in parallel, across collections, or through bounded loops. Orbit's product story is moving toward goals, graphs, sessions, and locks, but this layer remains the runtime underneath. [2_design.md](./2_design.md) is the current contract; [3_vision.md](./3_vision.md) captures open questions.

> **v1 release scope.** v1 ships `backend: cli` as the supported agent invocation path. HTTP `LoopTransport` (`backend: http`) and Groundhog exist in code and tests, but remain preview-only until v2.

---

## 1. Motivation

Orbit needs a runtime layer that humans can inspect and code can execute. Activity / Job solves four practical problems:

1. **Typed execution.** Agent loops, deterministic actions, shell steps, and Groundhog attempts share one schema family after [T20260418-2010].
2. **Durable local control flow.** Retry, parallelism, fan-out, and loops survive outside one model turn via `JobV2` DAG constructs from [T20260418-2018].
3. **Clean runtime boundaries.** orbit-core coordinates runs without naming `orbit-agent` internals through the `V2RuntimeHost` work in [T20260418-2143] and [T20260418-2210].
4. **One canonical schema.** `schemaVersion: 1` assets fail load-time parsing after [T20260419-2156].

---

## 2. Core Concepts

### 2.1 Activities are the runnable units

An `ActivityV2` carries shared metadata plus one runtime spec:

- `agent_loop`
- `groundhog`
- `deterministic`
- `shell`

The shared shape shipped in [T20260418-2010]. Groundhog became a sibling activity kind in [T20260420-0510-2], not another `agent_loop` flag.

### 2.2 Jobs are the orchestration grammar

A `JobV2` is a step tree with:

- `when`
- `retry`
- flat target steps
- `target: activity:<name>` references
- `parallel`
- `fan_out` / `fan_in`
- `loop`

That grammar landed in [T20260418-2018], with `workflow` / `subroutine` job kinds added in [T20260419-0339].

### 2.3 Load-time normalization is part of the contract

orbit-core normalizes assets before a run starts:

- loads YAML through a two-pass schema loader
- resolves `target: activity:<name>` references for jobs
- rewrites `backend: auto` to a concrete backend once per run
- rejects loop/session/backend combinations that cannot execute safely

Name resolution arrived in [T20260418-2019]. Backend resolution and `run-v2` entrypoints came in [T20260418-2143]; CLI backend support followed in [T20260419-0104].

### 2.4 Backends and providers are separate choices

For `agent_loop`, Orbit distinguishes:

- **backend**: `http`, `cli`, or `auto`
- **provider**: `claude`, `codex`, `gemini`, `ollama`, or `openai_compat`

`backend: auto` resolves once at load time. `backend: http` against an unwired provider fails structurally instead of falling back. `backend: cli` intentionally retains the older CLI-provider runtimes per [T20260419-0104] and [T20260418-2210].

### 2.5 Audit, policy, and seeded assets make the runtime inspectable

This layer also owns:

- `fsProfile` attachment on activities and target steps
- the v2 audit envelope with `workspace_path` provenance
- seeded reference assets and pipeline jobs used by `orbit init`

`workspace_path` entered the envelope in [T20260419-0002], runtime/CLI `fsProfile` enforcement landed in [T20260419-0503], and init seeding landed in [T20260419-2347].

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| v2 activity type system | `crates/orbit-common/src/types/activity_job/activity_v2.rs` | [T20260418-2010] |
| v2 job step grammar | `crates/orbit-common/src/types/activity_job/job_v2.rs` | [T20260418-2018] |
| Job kinds (`workflow`, `subroutine`) | `crates/orbit-common/src/types/activity_job/job_v2.rs` | [T20260419-0339] |
| Target-ref resolution | `crates/orbit-common/src/types/activity_job/catalog.rs` | [T20260418-2019] |
| `run-v2` core entrypoints and host boundary | `crates/orbit-core/src/command/activity_v2.rs`, `crates/orbit-core/src/command/job_v2.rs` | [T20260418-2143], [T20260418-2210] |
| Backend resolution and loop/session constraints | `crates/orbit-core/src/command/backend_resolver.rs`, `crates/orbit-common/src/types/activity_job/backend.rs` | [T20260419-0104] |
| v2 DAG executor | `crates/orbit-engine/src/activity_job/job_executor.rs` | [T20260418-2018] |
| V2 audit envelope and disk sink | `crates/orbit-common/src/types/activity_job/audit_envelope.rs`, `crates/orbit-engine/src/activity_job/audit_writer.rs` | [T20260419-0002] |
| `backend: cli` runtime path | `crates/orbit-engine/src/activity_job/cli_runner.rs` | [T20260419-0104] |
| `fsProfile` enforcement | `crates/orbit-policy`, `tool_context_for_activity`, CLI describe/get surfaces | [T20260419-0503] |
| Seeded reference activities and pipeline jobs | `crates/orbit-core/assets/activities/`, `crates/orbit-core/assets/jobs/` | [T20260419-2347], [T20260419-0622-3], [T20260419-0623], [T20260419-0623-2] |
| Groundhog as a sibling activity kind | `crates/orbit-engine/src/activity_job/groundhog.rs` | [T20260420-0510-2] |

---

## Task References

- **[T20260418-2010]** — Add the first v2 activity runtime scaffolding.
- **[T20260418-2018]** — Add `JobV2` DAG constructs (`parallel`, `fan_out`, `loop`, `retry`, `when`).
- **[T20260418-2019]** — Add v2 activity name resolution and pipeline skeleton assets.
- **[T20260418-2143]** — Wire `V2RuntimeHost` in orbit-core and add `orbit activity run-v2`.
- **[T20260418-2210]** — Reshape `V2RuntimeHost` to keep `orbit-agent` types out of orbit-core.
- **[T20260419-0002]** — Add `workspace_path` provenance to the v2 audit envelope.
- **[T20260419-0104]** — Add `backend: cli` dispatch for v2 `agent_loop`.
- **[T20260419-0339]** — Add v2 job kinds to the job catalog.
- **[T20260419-0503]** — Enforce `fsProfile` rules across runtime and CLI surfaces.
- **[T20260419-0622-3]** — Add `task_gate_pipeline`.
- **[T20260419-0623]** — Add `task_auto_pipeline`.
- **[T20260419-0623-2]** — Add `task_epic_pipeline`.
- **[T20260419-2156]** — Retire v1 assets and drop the transitional v2 naming.
- **[T20260419-2347]** — Seed activities and workflows on `orbit init`.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260430-19]** — Shorten the Activity / Job design docs while preserving required structure.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

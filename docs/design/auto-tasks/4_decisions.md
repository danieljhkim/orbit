---
title: Auto-tasks — Decisions
owner: claude
last_updated: 2026-08-11
last_validated: 2026-08-29
status: Accepted
feature: auto-tasks
doc_role: decisions
type: design
summary: Decision log for the auto-task primitive.
tags: [auto-tasks]
paths: ["crates/orbit-core/src/auto_tasks/**"]
related_features: [auto-tasks]
related_artifacts: []
---

# Auto-tasks — Decisions

This document preserves the feature's non-obvious decisions and their reasoning.

---

## Auto-task primitive: file-backed recurring task templates + one generic scheduler routine

**Recorded:** 2026-07-12 02:58:04.684957Z · [ORB-10149], [ORB-10148]
**Paths:** `crates/orbit-core/src/auto_tasks/**`

### Context

Every periodic need in Orbit was previously bespoke code, and each future recurring chore meant another hardcoded routine. The marginal cost of a periodic chore was therefore a code change, review, and release.

### Decision

Introduce auto-tasks as git-versioned YAML definitions under `.orbit/auto_tasks/<name>.yaml`, with cron/interval schedules, host-local cursors, task templates, dedupe, and provenance. One generic deterministic scheduler activity wrapped in a job and fired by the seeded routine processes every definition. Definitions parse fail-closed, catch-up collapses, and CRUD is available through CLI and MCP. Templates remain provider-neutral per [Run budgets are provider-neutral: wall-clock timeouts, never turn caps](#run-budgets-are-provider-neutral-wall-clock-timeouts-never-turn-caps).

### Consequences

- Periodic work becomes data; QA sweep is the first checked-in definition.
- Definitions are workspace-scoped and scheduler fires remain observable through routine health.
- Host-local cursor state avoids churn in git-versioned definitions.
- Cost: a second file-backed record convention exists alongside the SQLite-indexed knowledge records, and auto-task definitions are not full-text indexed.

## No-diff-expected tasks bypass repository change gates

**Recorded:** 2026-07-12 03:33:35.554901Z · [ORB-10148]
**Paths:** `crates/orbit-engine/src/executor/automation/vcs/**`, `crates/orbit-types/src/task/model.rs`, `.orbit/auto_tasks/**`

### Context

Some normal workflow tasks produce durable side effects through Orbit rather than repository changes. QA validation files follow-up tasks and may correctly leave the worktree unchanged; treating every empty diff as an implementation failure strands valid runs, while weakening the gate globally would hide broken implementation tasks.

### Decision

`no-diff-expected` is a first-class task tag. The commit and PR handoff gates bypass empty-stage and zero-commits-ahead failures only when the relevant task (or every task in a PR bundle) carries the tag; the run still requires a meaningful execution summary and advances through the normal lifecycle without creating an empty commit or PR.

### Consequences

- Side-effect-only validation tasks can complete through normal orchestrator dispatch.
- Ordinary implementation tasks retain fail-closed empty-diff checks.
- The checked-in QA auto-task template carries the exemption explicitly, keeping the exception visible in data.
- Cost: a mistagged task can reach review without repository changes, so definition authors and reviewers must treat this tag as a privileged workflow exemption.

## Route tracked auto-task definitions through the active worktree

**Recorded:** 2026-08-08 19:11:08.591438Z · [ORB-10472]
**Paths:** `crates/orbit-core/src/auto_tasks/**`, `crates/orbit-engine/src/activity_job/workspace.rs`

### Context
Auto-task definition CRUD used the shared Orbit root even when a workflow executor was assigned a linked worktree. That let a refresh expose tracked dirt in the registered primary checkout while unrelated implementation guards were sampling it. The alternatives were to tolerate auto_tasks drift in the integrity guard or to make tracked definition mutation worktree-local.

### Decision
Read and replace tracked auto-task definitions through the runtime local root. Keep scheduler cursors and coordination state under the shared root, and replace each definition atomically so a failed refresh cannot expose partial bytes.

### Consequences
- Linked-worktree refreshes become ordinary branch changes and concurrent implementation guards continue to observe an unchanged primary checkout.
- Primary-checkout operator commands retain existing behavior because local and shared roots are identical there.
- Cost: callers in linked worktrees see that checkout version of definition YAML while scheduler cursor state remains shared, so definition and cursor roots must remain deliberately separate.

## Run budgets are provider-neutral: wall-clock timeouts, never turn caps

**Recorded:** 2026-07-12 03:33:34.766432Z · [ORB-10146], [ORB-10148]
**Paths:** `crates/orbit-core/assets/**`, `crates/orbit-types/src/workflow/auto_task.rs`, `.orbit/auto_tasks/**`

### Context

Orbit dispatches agent runs across provider families through named crews, and any crew can be assigned to any lane. Turn caps are not portable: provider CLIs expose different controls and semantics. Real alternatives were to retain optional provider-specific turn budgets, or forbid them as cross-provider policy and use neutral limits.

### Decision

All run budgets in Orbit config, auto-task definitions, job/activity assets, workflow invoke payloads, and task specs are provider-neutral: wall-clock timeout is the primary bound, with neutral resource caps where needed. Turn caps (`max_turns` or equivalents) do not appear on these surfaces. Provider-specific throttles may exist only inside one provider adapter as implementation detail.

### Consequences

- Swapping a crew never changes configured budget semantics.
- Config and job assets stay portable across crews without provider conditionals in dispatch paths.
- Existing turn-based policy knobs must be retired or demoted to adapter-internal defaults.
- Cost: Orbit gives up fine-grained turn limits exposed by individual providers; a looping run is bounded by neutral time/resource limits instead.

## Task References

- [ORB-10149] — Shipped the auto-task primitive (record, scheduler, CRUD, assets).
- [ORB-10148] — Added the QA definition and no-diff workflow exemption.
- [ORB-10472] — Isolated auto-task definition refresh from the registered
  primary checkout.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

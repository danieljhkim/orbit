# Groundhog — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-10 (heartbeat drift review)

This document describes Groundhog as it exists in the codebase today: the activity shape, plan schema, persisted artifacts, attempt lifecycle, builtin verbs, verifier path, and the current implementation gaps that still separate the runner from the intended v1 contract. See [1_overview.md](./1_overview.md) for the feature's purpose and [3_vision.md](./3_vision.md) for forward-looking questions.

---

## 1. Activity and Plan Surface

Groundhog is a dedicated v2 activity kind, not a flag on `agent_loop`. `ActivityV2Spec::Groundhog` carries its own `GroundhogSpec` with `instruction`, `tools`, `on_denial`, `model`, `max_iterations`, `provider`, `wall_clock_timeout_seconds`, and `attempt_budget_default`. That activity shape shipped in [T20260420-0510-2].

The dispatcher treats Groundhog as HTTP-only in practice: it rejects providers whose HTTP transport is not wired and routes `GroundhogSpec` through a dedicated `run_groundhog_activity` entry point. The spec currently does not expose a separate `plan_source`; the runner always loads checkpoints from the task's stored `plan` field.

Checkpoint structure comes from `TaskPlan` in `crates/orbit-common/src/types/task_plan.rs`, added in [T20260420-0509-2]. Each checkpoint carries:

- `id`
- `spec`
- typed `success_criteria`
- `attempt_budget`

The parser assigns a default per-checkpoint `attempt_budget` of `3` when the plan omits one. The Groundhog runner then applies `effective_attempt_budget(checkpoint, spec.attempt_budget_default)`, which currently behaves as a floor (`max`) rather than a true fallback.

---

## 2. Persisted State and Artifacts

Groundhog currently persists two task artifacts:

- `artifacts.chronicle`
- `groundhog/state.json`

The chronicle lives in `orbit_common::groundhog` and was introduced in [T20260420-0509]. Its current persisted vocabulary is:

- `Chronicle`
- `Day`
- `DayOutcome`
- `Attempt`
- `FailureReport`
- `SideEffect`
- `ToolCallRecord`

The runner state in `groundhog/state.json` was added by [T20260420-0510-2]. It tracks:

- `next_snapshot_n`
- the currently active checkpoint, if any
- the active checkpoint's accumulated attempts
- the latest failure report for retry context

This means the current implementation does not yet persist the cleaner split described by the intended v1 contract (`GroundhogMemory` for prompt-facing state and `GroundhogRun` for audit-only state). Instead, prompt-facing memory is reconstructed from successful `Day` entries in the chronicle, and retry bookkeeping sits beside it in the runner-state artifact.

One more legacy detail matters: the chronicle type still carries `deviation_stack`, and `DayOutcome` still has a `DeviatedTo` variant. The current v1 runner does not use either, but the persisted type shape still reflects older Groundhog drafts.

---

## 3. Attempt Lifecycle and Prompt Construction

The dedicated runner is rooted at `crates/orbit-engine/src/activity_job/groundhog/mod.rs`, with attempt handling, persistence/state, and the local verifier in sibling submodules. The original runner shipped in [T20260420-0510-2], and the focused module split landed in [T20260509-19]. Its lifecycle today is:

1. Load the task via `orbit.task.show`.
2. Parse the task's structured plan.
3. Resolve the workspace path from input, task metadata, or tool context.
4. Load `artifacts.chronicle` and `groundhog/state.json`.
5. Determine the active checkpoint.
6. Create a `WorkspaceSnapshot`.
7. Run one attempt with a fresh `AttemptGroundhogHost`.
8. On terminal success, verify the checkpoint and either commit or rewind.
9. On terminal failure, rewind and either retry or abandon.

Each attempt gets a prompt built from:

- the full raw task plan
- summaries of prior successful checkpoint records in the chronicle
- the current checkpoint `id`, `spec`, and `success_criteria`
- the latest `FailureReport` for the active checkpoint, if retrying

The Groundhog host records three kinds of builtins during the attempt:

- side effects
- checkpoint success
- checkpoint failure

If the loop ends without a Groundhog terminal builtin, the runner synthesizes a `FailureReport` and treats the attempt as failed. This keeps the retry path deterministic even when the agent stops talking instead of closing the attempt explicitly.

Current prompt memory is still summary-only. Although successful `Day` records store `side_effects`, the prompt builder does not replay those side-effect summaries into later attempts yet.

---

## 4. Workspace Snapshot and Commit Model

The git-backed snapshot helper lives in `crates/orbit-engine/src/workspace_snapshot.rs` and shipped in [T20260420-0509-4]. Its contract is described in [specs/workspace-snapshot.md](./specs/workspace-snapshot.md).

The key runtime behavior today is:

- Groundhog requires a named task branch; detached HEAD fails immediately.
- The tracked workspace must be clean before snapshot creation.
- Each attempt creates a scratch branch named `groundhog/<task_id>/day-<n>`.
- Pre-existing untracked files are preserved across the attempt.
- `rewind` captures the scratch branch state, checks out the task branch, and `git reset --hard`s back to `snapshot_ref`.
- `commit_success` captures the scratch branch state, resets the task branch to `snapshot_ref`, squash-merges the scratch branch, creates one commit from the checkpoint summary, and deletes the scratch branch.

This is a strong current implementation choice: successful checkpoints land directly on the task branch. The code does not yet support the "internal Groundhog-managed ref first, materialize later" path described by the intended v1 design.

---

## 5. Builtin Tool Surface

Groundhog-specific tools landed in [T20260420-0509-3]. The runner force-injects the required Groundhog verbs into the attempt allowlist:

- `orbit.groundhog.checkpoint_success`
- `orbit.groundhog.checkpoint_failure`
- `orbit.groundhog.side_effect`

These builtins are only legal when `ToolContext.groundhog_host` is present and the runner marks the scope as an active Groundhog day. The payloads are:

- success: `{summary, side_effects}`
- failure: `{what_tried, what_happened, next_attempt_plan}`
- side effect: `{kind, target, reversible}`

The legacy `orbit.groundhog.checkpoint_deviate` verb is no longer registered in the public tool surface as of [T20260426-0603]. Some internal deviation types still exist as deferred cleanup substrate, but Groundhog v1 exposes only success, failure, and side-effect verbs to attempts.

---

## 6. Verification Path

The shared verifier module in `crates/orbit-engine/src/checkpoint_verifier.rs` landed in [T20260420-0510]. It defines:

- `Criterion`
- `CriterionRun`
- `CriterionOutcome`
- `VerifierResult`

and it evaluates criteria in parallel.

The Groundhog runner, however, does not currently call that shared verifier. `crates/orbit-engine/src/activity_job/groundhog/verifier.rs` still carries a local `verify_checkpoint(...)` helper that:

- evaluates criteria sequentially
- returns only an optional `FailureReport`
- uses plain substring matching for `file_contains`
- does not persist verifier runs on pass or fail

So the codebase already contains the richer verifier surface, but the Groundhog activity path still uses a thinner local verifier. This is one of the main current mismatches between Groundhog's intended v1 contract and its present implementation.

---

## 7. Concerns & Honest Limitations

This section is the active gap ledger. Keep shipped behavior in the mechanism sections above, and keep cleanup, drift, and decision pressure here until a task or ADR resolves them.

### 7.1 Persistence still reflects older Groundhog vocabulary

The current runtime persists `Chronicle` + `groundhog/state.json`, not a cleanly separated `GroundhogMemory` + `GroundhogRun`. That is serviceable, but it mixes prompt-facing and audit concerns more than the intended v1 shape.

### 7.2 Attempt audit fidelity is still incomplete

`Attempt.tool_calls` exists in the persisted type, but the Groundhog runner currently pushes empty vectors. Attempt records also omit `scratch_branch`, `verifier_runs`, and the committed ref for successful checkpoints. Review/debug surfaces therefore have less fidelity than the design intends.

### 7.3 Prompt memory is narrower than the design target

Later attempts currently receive successful checkpoint summaries only. Side-effect summaries are persisted but not reloaded into the prompt. That means Groundhog remembers less about irreversible or notable prior changes than it claims to.

### 7.4 Legacy deviation substrate still ships

The current runner does not support deviation as part of Groundhog v1, and the public `checkpoint_deviate` verb has already been removed from the registered tool surface. What still remains is the older substrate: chronicle types, serializer vocabulary, tests, and internal modules that preserve deviation-era shapes. That cleanup is still worth doing because it keeps the persisted model and source tree more confusing than the shipped Groundhog contract needs them to be.

### 7.5 `attempt_budget_default` is not a true fallback yet

The parser gives every checkpoint an explicit budget, and the runner applies the activity default with `max(...)`. In practice that makes the activity-level value a floor, not a fallback. The docs need to name that honestly until the semantics are cleaned up.

### 7.6 Successful checkpoints commit directly to the task branch

This is operationally simple and matches the current code, but it leaves no approval-safe materialization layer for environments that want Groundhog commits to stay hidden until a later lifecycle boundary.

### 7.7 Provider wiring is narrower than the type surface suggests

`GroundhogSpec` carries a provider enum, but the runner currently resolves its API key through `api_key_for("anthropic")`. The dispatcher's HTTP transport gate keeps this mostly safe in practice, but the runtime path is still less provider-generic than the type surface implies.

### 7.8 Observability is still thin

The runner can report success, blocked status, and checkpoint counts through its activity output, but it does not yet emit the richer Groundhog-specific metrics the design wants: attempts per checkpoint, verifier pass/fail counts, scratch-branch lineage, or a read-only Groundhog chronicle view.

---

## Task References

- **[T20260420-0509]** — Add Groundhog chronicle serializer and shared Groundhog data types.
- **[T20260420-0509-2]** — Add structured task plan parsing with typed checkpoints and success criteria.
- **[T20260420-0509-3]** — Add Groundhog builtin verb tools.
- **[T20260420-0509-4]** — Add Groundhog workspace snapshots and scratch-branch rewind mechanics.
- **[T20260420-0510]** — Add the shared runtime checkpoint verifier.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260426-0603]** — Remove the public Groundhog checkpoint deviation verb from the tool surface.
- **[T20260430-21]** — Shorten Groundhog design docs and fold the implementation status ledger into numbered docs.
- **[T20260509-19]** — Split the Groundhog activity runner into focused engine submodules.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

# Groundhog — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-10 (aligned ADR metadata with recent Groundhog changes)

This ADR log records the design choices that define Groundhog's current shape. Entries stay in place when they are superseded; new decisions append at the end. See [1_overview.md](./1_overview.md) for the feature summary, [2_design.md](./2_design.md) for the current implementation, and [3_vision.md](./3_vision.md) for open questions that may force new ADRs.

---

## ADR-001 — Dedicated Groundhog activity kind

**Status:** Accepted · 2026-04 · [T20260420-0510-2]

**Context.** Groundhog has its own state, retry loop, and checkpoint-closing builtins. Treating it as an `agent_loop` toggle would have hidden that behavior inside flags and made dispatch harder to reason about.

**Decision.** Groundhog is its own `ActivityV2Spec::Groundhog` variant with a dedicated runner.

**Consequences.**
- Dispatch can validate Groundhog-specific preconditions up front.
- Runtime code gets a clear place to own checkpoint state and snapshot handling.
- Cost: one more activity shape to document, validate, and keep aligned with `agent_loop`.

## ADR-002 — HTTP-only first ship

**Status:** Accepted · 2026-04 · [T20260420-0510-2]

**Context.** Groundhog relies on a fresh prompt boundary per attempt and on explicit builtin closures. The existing CLI-backend path does not expose the same runtime control surface.

**Decision.** Groundhog's shipped runner is HTTP-only. Dispatch rejects providers whose HTTP transport is not wired.

**Consequences.**
- The first ship stays inside the transport model the runtime already controls.
- The provider/type surface remains narrower in practice than the enum implies.
- Cost: CLI-backed execution gets no Groundhog behavior unless the transport story broadens later.

## ADR-003 — Structured checkpoints live in the task plan

**Status:** Accepted · 2026-04 · [T20260420-0509-2], [T20260420-0510-2]

**Context.** Groundhog needs a durable, machine-readable checkpoint list. Freeform task plans do not give the runner enough structure to decide what to retry, verify, or record.

**Decision.** Groundhog reads typed checkpoints from the task's structured `plan` field.

**Consequences.**
- Checkpoint identity, success criteria, and retry budget are available to both runtime and agent.
- The task artifact becomes the authoritative source of execution structure.
- Cost: Groundhog inherits the quality of the task plan; weak checkpointing produces weak execution.

## ADR-004 — Git scratch branches for rewind

**Status:** Accepted · 2026-04 · [T20260420-0509-4]

**Context.** Retrying from a dirty workspace is the main failure mode Groundhog is trying to avoid. The rewind mechanism also needs to survive crashes and remain inspectable after a failed attempt.

**Decision.** Each attempt executes on a scratch branch named `groundhog/<task_id>/day-<n>` and rewinds by resetting the task branch back to `snapshot_ref`.

**Consequences.**
- Failed attempts leave behind inspectable scratch branches.
- Success can be materialized as one squash commit per checkpoint.
- Cost: scratch branches proliferate during long runs and need cleanup discipline.

## ADR-005 — Explicit Groundhog builtins close an attempt

**Status:** Accepted · 2026-04 · [T20260420-0509-3], [T20260420-0510-2]

**Context.** The runtime needs a crisp signal for "this attempt succeeded" versus "this attempt failed" without parsing freeform assistant text.

**Decision.** Groundhog uses dedicated builtins for checkpoint success, checkpoint failure, and side-effect recording. The runner treats missing terminal verbs as synthetic failure.

**Consequences.**
- Attempt closure is deterministic and machine-readable.
- Retry logic does not depend on assistant prose conventions.
- Cost: the tool surface becomes load-bearing; mismatches between docs and registered builtins are high-risk drift.

## ADR-006 — Preserve an append-only chronicle serializer

**Status:** Accepted · 2026-04 · [T20260420-0509]

**Context.** Groundhog wants stable checkpoint memory that can be serialized incrementally. Rewriting prior chronicle bytes would make cache-friendly prefix reuse impossible if the runtime ever leans on those helpers.

**Decision.** Keep an append-only chronicle serializer contract where earlier serializations are byte-exact prefixes of later ones.

**Consequences.**
- The runtime has a reusable primitive for stable checkpoint-memory serialization.
- Chronicle history can grow without mutating prior serialized bytes.
- Cost: current runtime persistence is still split across `Chronicle` and `groundhog/state.json`, so the serializer's benefits are only partially realized today.

## ADR-007 — Mechanical criteria verify at the checkpoint boundary

**Status:** Accepted · 2026-04 · [T20260420-0510], [T20260420-0510-2]

**Context.** Letting the agent self-certify success is too weak for buildable coding tasks. Mechanical checks need to execute outside the conversational loop.

**Decision.** Groundhog verifies mechanical success criteria at the checkpoint-success boundary and converts failures into retryable `FailureReport`s.

**Consequences.**
- Success is gated on workspace reality, not just agent confidence.
- A richer shared verifier can serve non-Groundhog code paths too.
- Cost: the current runner still uses its own thinner inline verifier, so this decision is only partially reflected in implementation.

## ADR-008 — Groundhog v1 excludes executor-authored deviation and retry critic

**Status:** Proposed · 2026-04 · [T20260420-0510-2], [T20260426-0603]

**Context.** Earlier Groundhog drafts centered deviation stacks and retry critics. The current v1 goal is narrower: prove checkpoint + rewind + verifier + success-memory before reopening more complex control flow.

**Decision.** Keep executor-authored deviation and critic-on-retry out of Groundhog v1. Revisit them only after the simpler loop has operational data behind it.

**Consequences.**
- The first shipped contract stays smaller and easier to reason about.
- Failure pressure shifts back to plan quality and explicit blocked outcomes.
- Cost: the current code still carries deviation-era leftovers, and v1 loses one potential escape hatch for bad plans.

## ADR-009 — Separate prompt-facing memory from audit record

**Status:** Proposed · 2026-04 · [T20260420-0509], [T20260420-0510-2]

**Context.** The current runtime persists a chronicle plus runner state. That works, but it does not cleanly separate "what later prompts should load" from "what operators should inspect after the fact."

**Decision.** The intended Groundhog direction is two persisted views: prompt-facing success memory and audit-only run history.

**Consequences.**
- Prompt loading rules become simpler and harder to accidentally violate.
- Audit surfaces can grow richer without bloating prompt state.
- Cost: migrating from today's `Chronicle` plus state artifact will require a persistence rewrite and compatibility plan.

---

## Task References

- **[T20260420-0509]** — Add Groundhog chronicle serializer and shared Groundhog data types.
- **[T20260420-0509-2]** — Add structured task plan parsing with typed checkpoints and success criteria.
- **[T20260420-0509-3]** — Add Groundhog builtin verb tools.
- **[T20260420-0509-4]** — Add Groundhog workspace snapshots and scratch-branch rewind mechanics.
- **[T20260420-0510]** — Add the shared runtime checkpoint verifier.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260426-0603]** — Remove the public Groundhog checkpoint deviation verb from the tool surface.
- **[T20260430-21]** — Shorten Groundhog design docs and add missing ADR task citations.
- **[T20260509-19]** — Split the Groundhog activity runner into focused engine submodules.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

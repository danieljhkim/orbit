---
summary: "Groundhog — Vision"
type: design
title: "Groundhog — Vision"
owner: codex
last_updated: 2026-04-30
status: Draft
feature: groundhog
doc_role: vision
tags: ["groundhog"]
---

# Groundhog — Vision

This document captures where Groundhog should go next. It starts from the implementation built in [T20260420-0509], [T20260420-0509-2], [T20260420-0509-3], [T20260420-0509-4], [T20260420-0510], and [T20260420-0510-2], and treats everything below as a hypothesis rather than a promise. [2_design.md](./2_design.md) is the current contract; this file is the place to pressure-test that contract before we harden more of it.

---

## 1. Open Questions

The highest-pressure questions are still deviation-era cleanup, shared-verifier adoption, persistence separation, approval-safe checkpoint materialization, and only-when-useful observability.

### 1.1 Separate prompt memory from audit record

Should Groundhog move fully to the intended split between prompt-facing memory and audit-only run state? The current `Chronicle` plus runner-state pair works, but it makes it too easy for prompt and audit concerns to bleed into each other.

### 1.2 Unify the runner on the shared verifier

The codebase already has a richer checkpoint verifier in `crates/orbit-engine/src/checkpoint_verifier.rs`. Should the Groundhog runner adopt it wholesale so the activity path and the shared verifier stop drifting?

### 1.3 Approval-safe checkpoint commits

Today success commits land directly on the task branch. Is that the right default forever, or should Groundhog materialize successful checkpoints onto internal refs until a later lifecycle step publishes them?

### 1.4 Plan source and planner interface

Should `GroundhogSpec` keep implicitly reading `task.plan`, or do we want an explicit `plan_source` contract and possibly a planner activity that produces checkpoints before execution begins?

### 1.5 Side-effect memory and irreversible actions

The design wants side-effect summaries to survive. How much of that should enter later prompts, and how strict should Groundhog be about disabling irreversible tools unless an activity explicitly opts in?

### 1.6 Observability and scoreboards

Which metrics actually matter for deciding whether Groundhog is working? Candidates include attempts per checkpoint, verifier pass/fail rates, rewind counts, blocked outcomes, and checkpoint-commit materialization latency. We should only add scoreboard surfaces that answer real operational questions.

### 1.7 Cache contract

The chronicle serializer preserves an append-only prefix property, but the current runtime does not yet operationalize cache breakpoints around Groundhog-specific prompt sections. How much cache behavior should be a real Groundhog contract versus a provider-specific optimization?

### 1.8 Controlled deviation as a follow-on

Older Groundhog drafts leaned heavily on executor-authored deviation. Should that return in a later version as a tightly controlled extension, or is "fail fast, fix the plan outside the runner" the healthier long-term discipline?

### 1.9 Critic-on-retry as a follow-on

The prior design explored a retry critic to combat cognitive entrenchment. Does Groundhog need that, or should we refuse to add a second agent until the simpler checkpoint + rewind + verifier loop has real failure data behind it?

### 1.10 Dedicated debug surface

Should Groundhog expose a read-only chronicle/debug view with checkpoint history, verifier summaries, and retained scratch branches, or is task-artifact inspection enough until the runner proves out?

---

## 2. Prior Work

Groundhog is a synthesis of known ideas, not a novel primitive. The interesting question is whether Orbit's combination is operationally useful, not whether each component is unprecedented.

### 2.1 Workflow checkpointing

- **LangGraph** treats checkpointing and durable execution as first-class workflow concerns.
- **Temporal** established the mental model of replayable workflow history and explicit activity boundaries.
- **Microsoft Agent Framework** has made checkpoints part of mainstream agent workflow vocabulary.

Groundhog borrows the core idea that work should resume from a named boundary, not from freeform conversational state.

### 2.2 Git-backed rollback

- **AgentGit** frames multi-agent work in version-control terms.
- **Aider** keeps agent edits tightly coupled to git and makes undo a first-class experience.
- **Plandex** leans on git-style plan branches and reviewable diffs.

Groundhog narrows this pattern to one need: checkpoint attempts should be rewindable without asking the agent to clean up after itself.

### 2.3 Context versioning and branching

- **Git Context Controller (GCC)** treats agent context itself as something like a branchable, mergeable workspace.

Groundhog's current v1 direction is intentionally simpler. It wants sequential checkpoints and explicit retry boundaries before it revisits richer context branching.

### 2.4 Reflection and retry

- **Reflexion** showed that retries plus distilled failure summaries can outperform single-shot execution.
- **MAR (Multi-Agent Reflexion)** highlighted the risk of cognitive entrenchment when the same agent critiques its own failed plan.

These are directly relevant to Groundhog's retry loop, even if Groundhog chooses not to ship a retry critic in v1.

### 2.5 Plan-driven coding agents

- **Plandex** is the closest practical analog in shape: plan-oriented coding work with git-native state and explicit subtask boundaries.

Groundhog differs mainly in scope. It is designed to be a runtime primitive inside Orbit's task/activity system, not a standalone interactive coding workflow.

---

## 3. What May Be Distinctive

Soft claims only:

- **Checkpoint plans as Orbit task data.** Groundhog treats checkpoint structure as part of the task artifact itself, not as ad hoc prompt scaffolding rebuilt every run.
- **Git scratch branches plus explicit terminal verbs.** The combination of scratch-branch rewind and builtin success/failure closure gives the runtime a crisp place to verify and persist outcomes.
- **Success-only memory as the default ambition.** The long-term target is not "remember the whole conversation better"; it is "remember only the successful checkpoint summaries and the one failure report that matters right now."

None of these amount to a research contribution. If Groundhog earns its keep, it will be because the discipline improves Orbit task execution in practice, not because the ingredients are unfamiliar.

---

## 4. References

### Orbit-internal

- [1_overview.md](./1_overview.md) — feature purpose and core concepts
- [2_design.md](./2_design.md) — current implementation
- [4_decisions.md](./4_decisions.md) — ADR log
- [specs/workspace-snapshot.md](./specs/workspace-snapshot.md) — git snapshot and rewind contract
- [../activity-job/2_design.md](../activity-job/2_design.md) — activity/job model consumed by the Groundhog runner

### External

- LangGraph — https://langchain-ai.github.io/langgraph/
- Temporal — https://temporal.io/
- Microsoft Agent Framework — https://learn.microsoft.com/
- Reflexion — https://arxiv.org/abs/2303.11366
- MAR: Multi-Agent Reflexion Improves Reasoning Abilities in LLMs — https://arxiv.org/abs/2512.20845
- AgentGit — https://arxiv.org/abs/2511.00628
- Git Context Controller — https://arxiv.org/abs/2508.00031
- Plandex — https://github.com/plandex-ai/plandex

---

## Task References

- **[T20260420-0509]** — Add Groundhog chronicle serializer and shared Groundhog data types.
- **[T20260420-0509-2]** — Add structured task plan parsing with typed checkpoints and success criteria.
- **[T20260420-0509-3]** — Add Groundhog builtin verb tools.
- **[T20260420-0509-4]** — Add Groundhog workspace snapshots and scratch-branch rewind mechanics.
- **[T20260420-0510]** — Add the shared runtime checkpoint verifier.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260430-21]** — Shorten Groundhog design docs and fold status-priority notes into numbered docs.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

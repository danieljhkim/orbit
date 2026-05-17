# Agent Families — Vision

**Status:** Draft
**Owner:** human
**Last updated:** 2026-05-16

The long-term direction is to keep family discovery small and explicit while moving experiment-friendly model selection into named crew configuration. Future work should reuse the registry instead of reintroducing hardcoded role defaults.

## 1. Open Questions

1. Should duel-plan participant selection read directly from the crew registry, or should it use a separate experiment matrix that references crews by name?
2. When a user wants to replace only the planner or reviewer for a task, should Orbit add per-role task overrides or require defining another named crew?
3. Should Orbit eventually version crew definitions so run-start can detect that a task pinned an older semantic lineup?
4. How much validation should occur at task authoring time versus run-start when remote config stores are introduced?

## 2. Prior Work

### Agent Family Onboarding

[ORB-00042] and follow-up tasks made `grok` a peer of `claude`, `codex`, and `gemini`. That work established the rule that adding a family is explicit and reviewed, not inferred from arbitrary strings.

### Role Defaults

Before [ORB-00058], default role models were split between `[agent.<role>]` config and hardcoded family pair resolution. That made model experiments global and temporary, which in turn made audit trails harder to interpret.

### Task Metadata

Task artifacts already supported optional fields that can be omitted from older YAML. The `crew` field follows that pattern so old tasks load while new tasks can express an intended lineup.

## 3. What May Be Distinctive

Orbit treats crew resolution as part of task execution provenance rather than just configuration lookup. A run records the exact resolved crew and role models so later readers can understand what actually ran even after the workspace registry changes.

The distinction between family and crew also lets future workflows ask two different questions cleanly: "which executors are supported?" and "which concrete lineup should this job use?"

## 4. References

- Orbit internal: [1_overview.md](./1_overview.md), [2_design.md](./2_design.md), [4_decisions.md](./4_decisions.md)
- Orbit internal: `crates/orbit-common/src/types/agent_pair.rs`
- Orbit internal: `crates/orbit-core/src/runtime/engine/crew.rs`

## Task References

- ORB-00042: Onboard Grok (xAI) as a first-class supported agent family.
- ORB-00058: Introduce per-task crew override for agent model selection.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

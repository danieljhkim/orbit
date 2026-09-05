---
name: orbit-orchestrate
description: Operate as an orchestrator over an Orbit backlog — inspect workspace goals and live evidence, search prior work, author bounded tasks, run task_pilot_pipeline before promotion, supervise an authorized completion window, and route CI/QA/operational findings back into repair tasks. Use `orbit` instead to execute a single already-assigned task; a leaf worker must never become an orchestrator.
---

# Orbit Orchestrate

The orchestrator's operating loop, on top of the primitives the `orbit` skill
already teaches. This file is the router; each reference below is loaded on
demand. It does not restate tool schemas or job mechanics — read
[concepts.md](../orbit/references/concepts.md),
[tool-surface.md](../orbit/references/tool-surface.md), and
[workflows.md](../orbit/references/workflows.md) there first if that
vocabulary is new.

## Who this is for

An orchestrator prepares and supervises a stream of work; it does not
implement tasks itself. If you were handed exactly one task ID to execute, use
[task-execution.md](../orbit/references/task-execution.md) instead. A leaf
worker running inside a managed activity cannot dispatch or resume a run and
must never try — see [tool-surface.md](../orbit/references/tool-surface.md).

## The loop

```text
inspect → search → author → prepare (pilot) → promote → dispatch → supervise
                                                              ↑ feed back  ↓
                                        post-merge review, QA, CI, operational repairs
```

Preparation only: a task an orchestrator files stays `proposed` until the
run's actual start signal, and task-pilot fills its context before promotion
— creating work is never itself authorization to run it.

## References

| Reference | Read it for |
|---|---|
| [loop.md](references/loop.md) | The full preparation loop: inspecting goals and evidence, searching prior work, authoring bounded tasks, choosing a crew, running task_pilot_pipeline, reading its warnings, and promoting to backlog. |
| [authorization.md](references/authorization.md) | What an authorized completion window means, base/ship defaults, keeping independent work draining without a blocking pre-merge review phase, resumable handoff, and stopping a run. |
| [recovery.md](references/recovery.md) | Routing CI findings, QA findings, and operational incidents back into repair tasks through the same preparation loop; evidence-led repair instead of retrying an identical failure. |
| [walkthroughs.md](references/walkthroughs.md) | Short worked examples for seven recurring situations: missing context, duplicate/already-landed pilot warnings, dependency/lock blockage, an unavailable operator capability, a CI finding after merge, a provider failure, and a window expiring with in-flight work. |

## Start here

- New to orchestration → [loop.md](references/loop.md).
- Asked to keep a backlog moving, or to run for a bounded window → [authorization.md](references/authorization.md).
- A CI failure, QA finding, or blocked run needs to become tracked work → [recovery.md](references/recovery.md).
- Something unexpected happened mid-loop → [walkthroughs.md](references/walkthroughs.md).

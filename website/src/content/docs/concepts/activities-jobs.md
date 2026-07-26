---
title: Activities and Jobs
description: "How Orbit represents executable work units and workflow orchestration."
sidebar:
  order: 3
---

## Activity

An activity is a reusable execution unit. Schema v2 activities declare `schemaVersion: 2`, `kind: Activity`, metadata, and a typed `spec`.

Supported activity types:

| Type | Use |
|------|-----|
| `agent_loop` | Run an agent with an instruction, provider, backend, and tool allowlist. The default backend is `cli`; `http` and `auto` are also accepted. |
| `deterministic` | Run a registered deterministic action. |

## Job

A job is a workflow. It has schedule state, optional default input, concurrency limits, and ordered steps.

Step bodies can reference an activity, inline an activity spec, or compose control flow:

- `target: activity:<name>`
- `spec: ...`
- `parallel`
- `fan_out` and `fan_in`
- `loop`

## Why Both Exist

Activities make execution behavior reusable. Jobs make orchestration explicit. This keeps the dispatch surface inspectable and avoids hiding agent behavior inside code.

**Example:** A job step referencing a reusable activity.

```yaml
# .orbit/activities/analyze_code.yaml
schemaVersion: 2
kind: Activity
name: analyze_code
spec:
  backend: cli
  provider: gemini
  model: gemini-3.1-pro
  instruction: "Analyze the provided code."

---
# .orbit/jobs/review_pr.yaml
schemaVersion: 2
kind: Job
name: review_pr
steps:
  - id: analysis
    target: activity:analyze_code
```

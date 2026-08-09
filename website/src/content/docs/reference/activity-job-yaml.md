---
title: Activity and Job YAML
description: "Reference shapes for schemaVersion 2 activity and job assets."
sidebar:
  order: 3
---

## Activity Envelope

```yaml
schemaVersion: 2
kind: Activity
metadata:
  name: example_activity
spec:
  type: deterministic
  description: Run a registered deterministic action.
  action: example_action
  input_schema_json:
    type: object
    properties: {}
  output_schema_json:
    type: object
    properties:
      status:
        type: string
```

## Activity Types

| Type | Required fields | Notes |
|------|-----------------|-------|
| `agent_loop` | `instruction`; optional `tools`, `provider`, `backend`, `model`, `max_iterations`, `wall_clock_timeout_seconds` | The backend defaults to `cli`; `http` and `auto` are also accepted. `max_iterations` applies only to HTTP loops, while `wall_clock_timeout_seconds` bounds CLI invocations. |
| `deterministic` | `action`; optional `config` | Runs a registered deterministic action. |

## Job Envelope

```yaml
schemaVersion: 2
kind: Job
metadata:
  name: example_job
spec:
  state: enabled
  max_active_runs: 1
  kind: workflow
  steps:
    - id: run_action
      target: activity:deterministic_reference
```

## Step Bodies

Reference an activity:

```yaml
- id: assess
  target: activity:agent_assess_diff
```

Inline a full activity spec:

```yaml
- id: run_action
  spec:
    type: deterministic
    action: example_action
    config: {}
```

Run branches in parallel:

```yaml
- id: parallel_assessment
  parallel:
    join: { mode: all }
    branches:
      - id: branch_a
        target: activity:assess_a
      - id: branch_b
        target: activity:assess_b
```

## Modifiers

Each step may include `when` and `retry`.

```yaml
retry:
  max_attempts: 3
  initial_backoff_ms: 500
  backoff_cap_ms: 5000
  backoff_strategy: exponential
```

---
title: Write an Activity
description: "Create a schemaVersion 2 activity file for agent, deterministic, or Groundhog execution."
sidebar:
  order: 3
---

## Start with the Header

Every activity uses this envelope:

```yaml
schemaVersion: 2
kind: Activity
metadata:
  name: deterministic_reference
spec:
  type: deterministic
  description: Run a registered deterministic action.
```

## Add Schemas

Use JSON Schema-shaped input and output declarations.

```yaml
input_schema_json:
  type: object
  properties: {}
output_schema_json:
  type: object
  properties:
    status:
      type: string
```

## Choose a Type

For a deterministic activity, name a registered action and pass optional config:

```yaml
type: deterministic
action: example_action
config: {}
```

For an agent loop, declare instruction, tools, provider, and backend. v1 supports `backend: cli` only:

```yaml
type: agent_loop
instruction: Review the current diff and report risks.
tools:
  - orbit.task.show
  - orbit.graph.search
provider: claude
backend: cli
max_iterations: 25
```

`backend: http` is wired in code for v2 but is not part of the v1 release surface — do not pin it in shipped activity assets.

## Use It

```bash
orbit activity list
orbit job run path/to/job.yaml --input key=value
```

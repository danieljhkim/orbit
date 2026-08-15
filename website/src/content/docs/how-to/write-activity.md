---
title: Write an Activity
description: "Create a schemaVersion 2 activity file for agent or deterministic execution."
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

For an agent loop, declare instruction, tools, and provider:

```yaml
type: agent_loop
instruction: Review the current diff and report risks.
tools:
  - orbit.task.show
  - orbit.search
provider: claude
```

Orbit dispatches every agent loop through the provider's CLI agent. There is no
backend to choose: the retired `backend:` key still parses as `cli` and is
ignored, while `backend: http` and `backend: auto` are refused at load.

## Use It

```bash
orbit activity list
orbit job run path/to/job.yaml --input key=value   # submits and returns a run ID
orbit job run path/to/job.yaml --wait              # block until the run is terminal
```

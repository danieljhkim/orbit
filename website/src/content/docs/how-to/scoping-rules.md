---
title: Choose Scopes
description: "Select the right state scope and filesystem profile for Orbit assets and execution."
sidebar:
  order: 4
---

## Artifact Scope

Orbit uses these scope strategies:

| Artifact | Strategy |
|----------|----------|
| Tasks | WorkspaceOnly |
| Activities and jobs | MergeByKey |
| Policies | MergeByKey |
| Job runs | WorkspaceOnly |
| Skills | MergeByKey |
| Audit | GlobalOnly |

Use workspace-local state for work tied to a repository. Use global state for shared defaults and the audit trail; skills use global defaults with optional workspace overrides by skill name.

## Filesystem Scope

Use `fsProfile` to select what an activity may read and modify.

```yaml
spec:
  type: agent_loop
  fsProfile: reviewer
```

Then define the profile in policy:

```yaml
fsProfiles:
  reviewer:
    read: [./**]
    modify: []
```

Global `denyRead` and `denyModify` rules still apply.

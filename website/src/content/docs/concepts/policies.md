---
title: Policies
description: "How Orbit uses filesystem profiles and global deny rules to scope execution."
sidebar:
  order: 4
---

## Definition

Policy is a filesystem-scoping surface. It controls what an activity can read or modify, then applies global deny rules on top.

An activity can select a named profile with `fsProfile`. If it omits the field, Orbit resolves an implicit unrestricted profile before global denies are applied.

> **Platform support.** OS-level enforcement of `fsProfile` for spawned agent CLIs uses macOS `sandbox-exec` and is **macOS only** today. On Linux and Windows the policy still applies as in-process FS guards for Orbit's HTTP-tool builtins, but the spawned agent subprocess runs without OS-level isolation.

## Shape

```yaml
schemaVersion: 2
kind: Policy
metadata:
  name: default
spec:
  denyRead:
    - "**/*.env"
  denyModify:
    - .orbit/**
    - "**/*.env"
  fsProfiles:
    reviewer:
      read: [./**]
      modify: []
```

## Use

Use narrow profiles for review and summarization. Use broader profiles only when
an agent is expected to edit code.

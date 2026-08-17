---
title: Policy Format
description: "Reference for Orbit policy YAML and filesystem profiles."
sidebar:
  order: 4
---

## Envelope

```yaml
schemaVersion: 2
kind: Policy
metadata:
  name: default
spec:
  denyRead: []
  denyModify: []
  fsProfiles: {}
```

## Global Denies

`denyRead` blocks reads. `denyModify` blocks writes. These rules accumulate globally and apply after the selected filesystem profile is resolved.

```yaml
denyRead:
  - "**/*.env"
denyModify:
  - .orbit/**
  - "**/*.env"
```

## Filesystem Profiles

Profiles describe allowed read and modify globs.

```yaml
fsProfiles:
  reviewer:
    read: [./**]
    modify: []
  implementer:
    read: [./**]
    modify:
      - crates/**
      - docs/**
```

An activity selects a profile with `fsProfile`.

```yaml
spec:
  type: agent_loop
  fsProfile: implementer
```

> **Platform support.** Spawned agent CLIs use a platform-specific OS boundary where supported: macOS uses `sandbox-exec`, and Linux uses trusted `/usr/bin/bwrap` after a namespace-and-mount capability probe. The Linux boundary enforces writes from the resolved profile while leaving host filesystem reads and host network access available; read rules and network-egress policy remain delegated. Linux dispatch fails closed when `/usr/bin/bwrap` is unavailable or the probe fails, unless the executor explicitly sets `allow_fallback: true`, which runs without Linux write confinement. On Windows and other unsupported platforms, the same policy YAML is applied to in-process FS-tool calls, but no OS-level backend wraps the agent subprocess.

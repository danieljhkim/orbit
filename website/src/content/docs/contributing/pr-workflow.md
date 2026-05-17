---
title: PR Workflow
description: "How to keep Orbit changes scoped, tested, and reviewable."
sidebar:
  order: 4
---

## Scope

Keep changes intentional. Avoid unrelated refactors. Update tests when behavior changes.

When a change touches an owned feature's implementation, update that feature's design docs in the same pull request. Flip affected ADR statuses, update the last-updated date, and add an ADR for non-obvious decisions.

## Checks

Run:

```bash
make fmt
make build
```

Use targeted tests while iterating and full workspace tests before landing risky changes.

## Commits

Use clear commit messages. Agent-authored commits should use the agent commit identity (e.g. `claude`, `codex`) for that commit and should not leave the repository configured with that identity afterward.

When a commit is associated with an Orbit task, include the task ID in square brackets in the commit message:

```text
feat: add optional agent_review step to bundle pipelines [T20260505-7]
```

When authoring tasks or design docs, identify yourself by agent family (`codex`, `claude`, `gemini`, or `grok`), not by a full model string. When writing docs, cite the task IDs that motivated the change in the doc itself.

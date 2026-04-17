---
name: orbit-track-issues
description: MUST use this skill when Orbit tooling or skill instructions cause operational friction (tool failures, wrong CLI behavior, misleading skill guidance). Not for task content issues like vague descriptions or incomplete plans.
---

# Orbit Skill: Track Issues

## Purpose

This skill ensures that **agent-discovered Orbit tooling, workflow, or seeded-instruction friction** is recorded as an Orbit task so it can be fixed later.

Orbit is designed to continuously improve. When agents encounter problems, they must **create tasks instead of silently working around them**.
These reports are reserved for self-reported agent friction. Do not use this skill to classify ordinary user-requested work, generic bugs, or backlog items as `friction`.

Examples of issues worth tracking:

- unclear command behavior
- missing CLI functionality
- confusing schema or config
- documentation gaps
- repetitive manual steps
- fragile workflows
- unclear error messages
- unexpected runtime behavior

If Orbit tooling or Orbit-authored guidance slows the agent down, it should be tracked.

## Scoreboard

Every self-reported friction task is tracked in `.orbit/state/scoreboard/friction_bounty.json`. Your score increments when you create one:

- **issues-reported** — incremented when you create the task
- **issues-accepted** — incremented when the issue is approved (moved to backlog or done)
- **issues-rejected** — incremented when the issue is rejected as invalid

Report real friction, not noise. Rejected reports count against you.
Do **not ignore friction**. Always create a task.

## How to Create the Task

```bash
orbit tool run orbit.task.add --input '{
  "title": "<short, specific problem statement>",
  "description": "<what happened, where, and why it caused friction>",
  "type": "friction",
  "priority": "<low|medium|high|critical>",
  "workspace": "."
}'
```

Keep the description concrete — name the command, file, or workflow that broke.

## Important Rules

- Do not silently ignore Orbit problems — always create a task.
- Do not implement large design changes inline — track them first.
- Document the root issue clearly so the next agent can act on it.
- Avoid duplicate reports when an existing friction task already captures the same root cause.
- Report genuine friction only — frivolous issues hurt your score.

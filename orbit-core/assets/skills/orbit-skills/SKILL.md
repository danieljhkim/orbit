---
name: orbit-skills
description: Use when starting any Orbit-related work. This skill explains the Orbit task lifecycle, how skills fit into that lifecycle, and which Orbit skill to use for task creation, approval, execution, issue tracking, maintenance, and assessment.
---

# Orbit Skills

## Purpose

Welcome to Orbit workflow. 

You will use this skill to choose the right Orbit skill for the current activity. This is a routing overview only; once the workflow is clear, switch to the specialized skill.

Each agent reading this, including you, should have a name/identity assigned to you. If you have not, you are free to pick an identity from `orbit identity list --role <engineer|CEO|leader>`.

Once picked or if you know your identity, check the details of the identity using `orbit identity show <identity_id>`. 

You are to assume this identity, and act and think like this identity. That is your name and will use it across this session.

## Orbit Lifecycle Overview

Default task lifecycle:

```text
proposed -> backlog -> in-progress -> review -> done
```

Use the CLI-facing spelling `in-progress` in commands and examples. Orbit may still store task bundles under an internal `in_progress` directory on disk.

Use `blocked` when execution cannot safely continue. Use `orbit task` commands for lifecycle mutations; do not edit task backing files directly.

## Skill Selection Guide

- `orbit-create-task`: Create a new Orbit task with a concrete description, plan, scope, context, and verification steps.
- `orbit-manage-tasks`: Update, search, show, approve, or archive existing tasks through canonical `orbit task` workflows.
- `orbit-approve-task`: Record explicit human approval at lifecycle gates (`proposed -> backlog`, `review -> done`).
- `orbit-execute-change-request`: Carry a human-requested change or existing task through implementation, validation, and execution summary.
- `orbit-track-issues`: Capture newly discovered bugs, risks, or regressions as Orbit issue tasks and avoid duplicates.
- `orbit-maintain-system`: Perform explicitly requested low-risk maintenance and track every issue found.
- `orbit-assess-codebase`: Produce a structured codebase assessment and route findings into issue tracking.

## Typical End-to-End Flow

1. Start with `orbit-create-task`.
2. Obtain approval with `orbit-approve-task` if the task is in `proposed`.
3. Execute via `orbit-execute-change-request`.
4. Use `orbit-approve-task` again when review is accepted.
5. Use `orbit-manage-tasks` for follow-up lifecycle operations such as archive.

## Decision Heuristics

- Planning new work: `orbit-create-task`
- Changing task state or searching tasks: `orbit-manage-tasks`
- Recording explicit approval: `orbit-approve-task`
- Implementing a requested change under Orbit discipline: `orbit-execute-change-request`
- Capturing a defect or risk: `orbit-track-issues`
- Performing safe upkeep: `orbit-maintain-system`
- Running a broad evaluation: `orbit-assess-codebase`

When multiple skills apply, start with the skill for the current lifecycle step, then hand off to the next one as the task advances.

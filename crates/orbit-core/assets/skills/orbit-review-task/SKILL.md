---
name: orbit-review-task
description: Use this when reviewing an Orbit task, PR, or branch and leaving inline feedback as review threads. Triggers on "review T-id", "review this PR", "leave review feedback", or any request to inspect changes and surface issues.
---

# Orbit Review Task

## Purpose

Review someone else's work and surface issues as Orbit review threads — without lifecycle transitions on the reviewed task. Reviewing is read + write-only-into-review-threads; it does not move the task forward.

## Tool Invocation

Both surfaces accept the same JSON. **`model` is required** for `review_thread.add` and `.reply`; the call rejects otherwise. Use your model name to score the review; pass `model: "human"` to opt out of scoring for human-attributed feedback.

```bash
orbit tool run orbit.task.review_thread.add --input '{
  "id": "<task-id>",
  "body": "<finding>",
  "path": "<repo-relative path>",   # optional, for inline review
  "line": <line>,                   # optional, requires path
  "model": "<your-model>"
}'

orbit tool run orbit.task.review_thread.reply --input '{
  "id": "<task-id>",
  "thread_id": "<thread-id>",
  "body": "<reply>",
  "model": "<your-model>"
}'
```

MCP form: `orbit_task_review_thread_add({...})` / `orbit_task_review_thread_reply({...})`.

## Workflow

### 1. Load context

Read the task with `orbit.task.show`. Pull the `description`, `acceptance_criteria`, `plan`, and `execution_summary`. Inspect the diff (`git diff` or PR view) and the changed files. Run `make build` and the relevant test target to confirm the change actually works.

### 2. Two-stage review

**Stage 1 — spec compliance** (do this first):
- Does the change satisfy every acceptance criterion?
- Anything missing? Anything added beyond what was asked? Any interpretation gaps?
- If spec compliance fails, file those threads and stop. Do not waste time on stage 2.

**Stage 2 — code quality** (only if spec compliance passes):
- Maintainability, patterns, performance.
- Test coverage gaps for the changed surface.
- Risks, edge cases, security concerns.

### 3. File review threads

One thread per distinct issue. Use inline (`path` + `line`) when the feedback ties to a specific location; use general (omit `path`/`line`) for cross-cutting concerns. Lead with a one-line headline (bold), then the why and the suggested fix. Keep each thread self-contained — reviewers should not need to read other threads to understand it.

```text
**[Spec compliance | Code quality | Nit] — short headline.**

Why this matters / what's wrong.

Suggested fix.
```

Tag severity in the headline so the implementer can triage: `Spec compliance`, `Code quality`, `Nit`. Spec compliance issues are blockers; nits are optional.

### 4. Summarize

End by reporting in chat: how many threads filed, which are blockers, overall verdict (approve / request changes). Do not add a `comment` to the task — review threads are the persistent surface; the chat summary is for the human running the review.

## Rules

- **Never** transition the reviewed task's status (no `orbit.task.update --status …`). The implementer or human owns lifecycle.
- **Never** resolve threads you authored — resolution is the implementer's call once they've addressed the feedback. Exception: if you replied to your own thread to retract the issue, resolve it.
- **Always** include `model` on every `review_thread.add` and `.reply` call. The tool rejects calls without it.
- **Replies don't score** — only the initial `review_thread.add` counts toward the local task-review scoreboard. Use replies for clarification, not for padding.
- If the change has no PR yet, review threads stay local. If it does have a PR, the GitHub sync flow will mirror them as PR review comments.

## When NOT to use this skill

- Implementing a task (use `orbit-execute-task`).
- Filing a friction report on Orbit tooling itself (use `orbit-track-issues`).
- Approving a task in `review` status (that's a lifecycle transition the reviewee or human performs via `orbit.task.approve`).

## Exit Criteria

- All findings filed as review threads with `model` attribution.
- No status transitions or thread resolutions performed on the reviewed task.
- Chat summary names blocker count and overall verdict.

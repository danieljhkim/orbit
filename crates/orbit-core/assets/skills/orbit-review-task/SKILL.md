---
name: orbit-review-task
description: Use this when reviewing an Orbit task, PR, or branch and leaving inline feedback as review threads. Triggers on "review T-id", "review this PR", "leave review feedback", or any request to inspect changes and surface issues.
---

# Orbit Review Task

## Purpose

Review someone else's work and surface issues as Orbit review threads — without lifecycle transitions on the reviewed task. Reviewing is read + write-only-into-review-threads; it does not move the task forward.

## Tool Invocation

Both surfaces accept the same JSON. **`model` is required** for `review_thread.add` and `.reply`; those calls reject without it. `review_thread.add` creates the scored finding, while `.reply` preserves attributed follow-up without creating a new finding. `review_thread.list` and `.resolve` do not require `model`, but the examples include it for consistent provenance. Use your agent family (`codex`, `claude`, `gemini`, or `grok`) for agent-attributed feedback; pass `model: "human"` to opt out of scoring for human-attributed feedback.

```bash
orbit tool run orbit.task.review_thread.add --input '{
  "id": "<task-id>",
  "body": "<finding>",
  "path": "<repo-relative path>",
  "line": "<line>",
  "model": "<agent-family>"
}'

orbit tool run orbit.task.review_thread.list --input '{
  "id": "<task-id>",
  "status": "open",
  "model": "<agent-family>"
}'

orbit tool run orbit.task.review_thread.reply --input '{
  "id": "<task-id>",
  "thread_id": "<thread-id>",
  "body": "<reply>",
  "model": "<agent-family>"
}'

orbit tool run orbit.task.review_thread.resolve --input '{
  "id": "<task-id>",
  "thread_id": "<thread-id>",
  "model": "<agent-family>"
}'
```

MCP form: `orbit_task_review_thread_add({...})` / `orbit_task_review_thread_reply({...})`.

## Workflow

### 1. Load context

Read the task with `orbit.task.show`. Pull the `description`, `acceptance_criteria`, `plan`, and `execution_summary`. Inspect the diff (`git diff` or PR view) and the changed files. Run `make build` and the relevant test target to confirm the change actually works.

Optional: if `orbit.semantic.*` is available, call `orbit.semantic.related` on the task ID to surface prior similar tasks whose decisions or review threads may inform this review. Skim snippets; ignore if no hit is relevant. No mandate — review threads remain grounded in the diff plus the task's own acceptance criteria. See `orbit-semantic`.

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

One thread per distinct issue. Use inline (`path` + string `line`) when the feedback ties to a specific location; use general (omit `path`/`line`) for cross-cutting concerns. Lead with a one-line headline (bold), then the why and the suggested fix. Keep each thread self-contained — reviewers should not need to read other threads to understand it.

```text
**[Spec compliance | Code quality | Nit] — short headline.**

Why this matters / what's wrong.

Suggested fix.
```

Tag severity in the headline so the implementer can triage: `Spec compliance`, `Code quality`, `Nit`. Spec compliance issues are blockers; nits are optional.

### 4. Summarize

End by reporting in chat: how many threads filed, which are blockers, overall verdict (approve / request changes). Do not add a `comment` to the task — review threads are the persistent surface; the chat summary is for the human running the review.

### 5. Meta-review (systemic prompt insufficiency)

After filing review threads, the reviewer checks whether the threads reveal a gap in an Orbit-authored agent instruction file (activity YAMLs under `crates/orbit-core/assets/activities/` such as `agent_implement.yaml` or `agent_review.yaml`, or skill `SKILL.md` files).

**Trigger heuristic:** two or more review threads in this session map to the same gap in an instruction asset, OR a single thread the reviewer recognizes as recurring / a class of issue that will recur on the next implementer task without a prompt change.

When the heuristic fires, file one friction via `orbit.friction.add` (MCP form: `orbit_friction_add`):

```bash
orbit tool run orbit.friction.add --input '{
  "body": "crates/orbit-core/assets/activities/agent_implement.yaml step 5 says \"implement only the task's scoped work\" but does not define scope-drift or require surfacing it as a comment when the implementer deletes unrelated code. Threads 1, 3, 7 were instances of this gap. Suggested language: \"If you delete or modify code outside the task's listed context files, surface the drift as a comment before continuing.\"",
  "tags": ["skill-guidance"],
  "during_task": "<task-under-review>",
  "model": "<reviewer-family>"
}'
```

Filing a friction is **additive** to filing individual review threads — it is not a replacement and the reviewer must still file threads on the actual code issues. The threads document what the implementer must fix; the friction is the meta-signal so a later task can strengthen the instruction asset.

**Negative cases — do not file a friction for:**
- A single nit
- A stylistic preference
- A one-off coding mistake with no link to instruction text

The bar (aligned with `orbit-track-issues` "report genuine friction only") is that the reviewer believes the class of issue would recur without an instruction change.

This step runs after Summarize and is the final action before exiting the review session.

## Rules

- **Never** transition the reviewed task's status (no `orbit.task.update --status …`). The implementer or human owns lifecycle.
- **Never** resolve threads you authored — resolution is the implementer's call once they've addressed the feedback. Exception: if you replied to your own thread to retract the issue, resolve it.
- **Always** include `model` on every `review_thread.add` and `.reply` call. The tool rejects calls without it.
- **Listing and resolution are not scored** — `review_thread.list` and `.resolve` do not require `model`; include it when you want consistent provenance in command history or resolver identity.
- **Replies don't create new findings** — only the initial `review_thread.add` counts toward the local task-review scoreboard. Use replies for clarification, not for padding.
- If the change has no PR yet, review threads stay local. If it does have a PR, the GitHub sync flow will mirror them as PR review comments.

## When NOT to use this skill

- Implementing a task (use `orbit-execute-task`).
- Filing a friction report on Orbit tooling itself (use `orbit-track-issues`). Filing a friction when review threads in aggregate reveal a systemic prompt insufficiency in an Orbit-authored agent instruction asset **is** in scope for the reviewer's session — the prior boundary is refined, not removed.
- Approving a task in `review` status (that's a lifecycle transition the reviewee or human performs via `orbit.task.approve`).

## Exit Criteria

- All findings filed as review threads with `model` attribution.
- No status transitions or thread resolutions performed on the reviewed task.
- Chat summary names blocker count and overall verdict.

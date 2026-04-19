---
name: orbit-approve-task
description: Use this skill when requested to review Orbit tasks for approval, record explicit human sign-off, or advance tasks through lifecycle gates, including explicit rejection.
---

# Orbit Approve Task

## Purpose

Use this skill to record explicit human approval or rejection after reviewing an Orbit task.

## Approval Gates

The tool commands auto-detect the current status and record the decision in task history:

- `proposed -> backlog`: appends `proposal_approved`
- `proposed -> rejected`: appends `proposal_rejected`
- `review -> done`: appends `review_approved`
- `review -> rejected`: appends `review_rejected`

When invoking `orbit tool run` directly, include `agent` and `model` in the input JSON:

```json
{
  "agent": "<claude|codex|gemini>",
  "model": "<model_name>"
}
```

```bash
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

## Workflow

1. Run `orbit tool run orbit.task.list --input '{"status": "proposed", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'` and `'{"status": "review", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`. If both are empty, your job is done.
2. For `proposed` tasks, review the task carefully.
   - If valid and execution should begin immediately: run `orbit tool run orbit.task.start --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`.
   - If valid but execution should remain queued: run `orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`.
   - If not valid: run `orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<reason>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`.
3. For `review` tasks, confirm all requirements were fulfilled as outlined in the task.
   - If complete and code changes are acceptable: run `orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`.
   - If incomplete: run `orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<what still needs resolving>", "comment": "<audit summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`.

## Verification

Prefer the returned payload from `approve`, `reject`, or `start` as your
default verification. If you need to confirm the canonical stored task record:

- Check status (all cases): `orbit tool run orbit.task.show --input '{"id": "<id>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`
- Check history (started case): `orbit tool run orbit.task.show --input '{"id": "<id>", "field": "history", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'`

Expected outcomes:

- After proposal approval: status is `backlog`
- After immediate proposal start: status is `in-progress`; history includes both `proposal_approved` and `started`
- After proposal rejection: status is `rejected`
- After review approval: status is `done`
- After review rejection: status is `rejected`

## Persisting Results

Include the brief audit summary in the same `approve`, `reject`, or `start` command via its `comment` field. That keeps the approval trail self-contained, even though `done` tasks can now accept follow-up comments later when you need to add cross-task notes.

```bash
orbit tool run orbit.task.approve --input '{
  "id": "<id>",
  "note": "<decision note>",
  "comment": "<action taken> — <decision note> — <verification result>",
  "agent": "<claude|codex|gemini>",
  "model": "<model_name>"
}'
```

Include: action taken (`proposal approved`, `proposal rejected`, `review approved`, or `review rejected`), task ID, and decision rationale. Keep comments concise, operational, and auditable.

## Safety Rules

- Never approve the wrong task ID.
- Never reject the wrong task ID.
- Never infer approval from ambiguity.
- Record meaningful notes for auditability.

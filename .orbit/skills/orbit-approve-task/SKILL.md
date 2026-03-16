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

```bash
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>"}'
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<note>"}'
```

## Workflow

1. Run `orbit tool run orbit.task.list --input '{"status": "proposed"}'` and `'{"status": "review"}'`. If both are empty, your job is done.
2. For `proposed` tasks, review the task carefully.
   - If valid: run `orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>"}'`. Also identify the best-suited engineer via `orbit tool run orbit.identity.list --input '{"role": "engineer"}'`.
   - If not valid: run `orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<reason>"}'`.
3. For `review` tasks, confirm all requirements were fulfilled as outlined in the task.
   - If complete and code changes are acceptable: run `orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>"}'` and include `result.commit` in the approval response so Orbit creates the commit.
   - A `review approved` result that accepts code changes must include `result.commit`; do not approve changed code without commit intent.
   - If incomplete: run `orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<what still needs resolving>"}'`.

## Verification

After each decision, run `orbit tool run orbit.task.show --input '{"id": "<id>"}'` and confirm:

- After proposal approval: status is `backlog`
- After proposal rejection: status is `rejected`
- After review approval: status is `done`
- After review rejection: status is `rejected`

## Output Requirements

Report:

- action taken (`proposal approved`, `proposal rejected`, `review approved`, or `review rejected`)
- task ID
- decision actor recorded by Orbit
- decision note
- verification result

Keep output concise, operational, and auditable.

## Safety Rules

- Never approve the wrong task ID.
- Never reject the wrong task ID.
- Never infer approval from ambiguity.
- Record meaningful notes for auditability.

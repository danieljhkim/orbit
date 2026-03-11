---
name: orbit-approve-task
description: Use this skill when requested to review Orbit tasks for approval, record explicit human sign-off, or advance tasks through lifecycle gates, including explicit rejection.
---

# Orbit Approve Task

## Purpose

Use this skill to record explicit human approval or rejection after reviewing an Orbit task.

## Approval Gates

The decision commands auto-detect the current status:

- `proposed -> backlog`: sets `proposal_approved_by` and `proposal_decision_note`
- `proposed -> archived`: sets `proposal_rejected_by` and `proposal_decision_note`
- `review -> done`: sets `review_approved_by` and `review_decision_note`
- `review -> backlog`: sets `review_rejected_by` and `review_decision_note`

Commands:

```bash
orbit task reject <id> --by "<identity_display_name>" --note "<note>" # reject
orbit task approve <id> --by "<identity_display_name>" --note "<note>" # approve
```

## Workflow

1. Find all tasks with status `proposed` or `review`.
2. Confirm all the tasks are in `proposed` or `review`.
3. For `proposed` tasks, review the task carefully and determine whether the task is valid.
   - If the proposed task is valid, approve with explicit identity and a meaningful note - and ASSIGN the task to the person (from `orbit identity list`) you think is best suited to complete it. 
   - If not valid, reject with `orbit task reject <id> --by <identity_display_name> --note <reason>`.
4. For `review` tasks, confirm all requirements were fulfilled as outlined in the task.
   - If the task is completed successfully, approve with `orbit task approve <id> --by <identity_display_name> --note <reason>`.
   - If the task is incomplete, reject with `orbit task reject <id> --by <identity_display_name> --note <reason>` and explain what still needs to be resolved.
5. If there was a code change and the workflow expects a commit, return commit intent for Orbit to execute instead of running `git commit` directly.
   - Put the request under `result.commit`
   - Required fields:
     - `message`: commit message string — must include the task ID (e.g. `"Fix foo bar [T20260310-062435-1773123875457583000]"`)
     - `files`: array of explicit file paths to stage

## Verification Rules

- Before decision: identity, scope, and current status are correct.
- After proposal approval: status is `backlog`.
- After proposal rejection: status is `archived`.
- After review approval: status is `done`.
- After review rejection: status is `backlog`.

## Output Requirements

Report:

- action taken (`proposal approved`, `proposal rejected`, `review approved`, or `review rejected`)
- task ID
- decision identity used
- decision note
- verification result
- If there are code changes, put the request under `result.commit`
   - Required fields:
      - `message`: commit message string — must include the task ID 
      - `files`: array of explicit file paths to stage

Keep output concise, operational, and auditable.

## Safety Rules

- Never approve the wrong task ID.
- Never reject the wrong task ID.
- Never infer approval from ambiguity.
- Prefer explicit `orbit task approve` and `orbit task reject` over implicit pathways.
- Record meaningful notes for auditability.

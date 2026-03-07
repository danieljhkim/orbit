---
name: orbit-approve-task
description: Use this skill when requested to review Orbit tasks for approval, record explicit human sign-off, or advance tasks through lifecycle gates.
---

# Orbit Approve Task

## Purpose

Provide a deterministic, auditable approval workflow for Orbit tasks at lifecycle gates (proposal approval and review approval).

## Task Lifecycle Gates

The `orbit task approve` command auto-detects the current task status:

- **Proposed → Backlog**: Sets `proposal_approved_by` and `proposal_decision_note`, moves task to `backlog`.
- **Review → Done**: Sets `review_approved_by` and `review_decision_note`, moves task to `done`.

## Task Commands

For canonical `orbit task` CLI workflows (show/approve/update/verify), refer to the `orbit-manage-tasks` skill.

This skill's approval-specific verification expectations:

- task identity and scope are correct
- current status is `proposed` or `review` (the two approvable states)
- approval fields are missing before approval, or present after approval

Expected after proposal approval:

- `proposal_approved_by` matches requested approver
- `proposal_decision_note` matches note (if provided)
- status is `backlog`

Expected after review approval:

- `review_approved_by` matches requested approver
- `review_decision_note` matches note (if provided)
- status is `done`

## Standard Workflow

### A) Explicit Human Approval

1. Inspect and confirm you have the correct task ID.
2. Approve with explicit approver identity and a meaningful note.
3. Verify approval metadata (`proposal_approved_by`/`review_approved_by` and corresponding note).

## Output Requirements

After approval actions, report:

- action taken (`proposal approved` or `review approved`)
- task ID
- approver identity used
- approval note (if set)
- verification status (approval fields present or reason not approved)

Keep output operational, concise, and auditable.

## Safety Rules

- Never approve the wrong task ID; always verify before mutation.
- Do not infer approval from ambiguity; require explicit confirmation.
- Prefer explicit `orbit task approve` over implicit pathways.
- Record meaningful notes for future auditability.

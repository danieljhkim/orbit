---
name: orbit-approve-task
description: Use this skill when requested to review Orbit tasks for approval, record explicit human sign-off, or allow agent execution after verbal approval.
---

# Approve Task

## Purpose

Provide a deterministic, auditable approval workflow for Orbit tasks before agent execution or implementation work begins.

## Scope

In scope:

- Review task readiness for approval
- Approve a task with explicit approver identity
- Record approval note and context
- Verify approval state after mutation
- Support verbal-approval execution path when explicitly confirmed by the user

Out of scope:

- Implementing the task itself
- Closing tasks without implementation validation
- Bulk policy overrides unless explicitly requested

## Approval Policy

Approval means the task is authorized for execution and includes traceable metadata:

- `approved_at`
- `approved_by`
- `approval_note` (optional but recommended)

If runtime config enforces approval gate:

```toml
[task.approval]
required_for_agent = true
```

then unapproved tasks must not proceed to agent execution.

## Command Reference

### 1) Inspect Task

```bash
orbit task show <TASK_ID>
orbit task show <TASK_ID> --json
```

Use this to confirm:

- task identity and scope are correct
- current status is not terminal (`done`/`cancelled`) unless explicitly intended
- approval fields are missing before approval, or present after approval

### 2) Approve Task (explicit sign-off)

```bash
orbit task approve <TASK_ID> --by "<approver>" --note "<approval rationale>"
```

Recommended note content:

- source of approval (ticket/review/verbal)
- constraints or guardrails
- any required follow-up checks

### 3) Verify Approval Mutation

```bash
orbit task show <TASK_ID> --json
```

Expected after approval:

- `approved_at` is populated
- `approved_by` matches requested approver
- `approval_note` matches note (if provided)

### 4) Verbal Approval Path (execution-time flexibility)

When the user explicitly confirms verbal approval in-session, this path is allowed:

```bash
orbit agent run --task <TASK_ID> --approve-on-verbal --approved-by "agent" --approval-note "approved based on explicit user verbal confirmation"
```

Use this only when verbal confirmation is explicit and unambiguous.

## Standard Workflow

### A) Explicit Human Approval

1. Read task with `orbit task show <TASK_ID>`.
2. Validate readiness and intent alignment.
3. Run `orbit task approve ...` with `--by` and optional `--note`.
4. Verify via `orbit task show <TASK_ID> --json`.
5. Report approval result concisely.

### B) Verbal Approval During Run

1. Confirm explicit user verbal approval in the conversation.
2. Run `orbit agent run --task <TASK_ID> --approve-on-verbal ...`.
3. Confirm task now contains approval metadata.
4. Report execution session result plus approval trace.

## Output Requirements

After approval actions, report:

- action taken (`approved`, `verified`, or `verbal-approval execution`)
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

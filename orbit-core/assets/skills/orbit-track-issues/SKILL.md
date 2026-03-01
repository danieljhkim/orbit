---
name: orbit-track-issues
description: Use this skill when issues are identified by agents or humans. All issues must be tracked. Use this to track issues properly. 
---

# Track Issues

## Purpose

Use this skill to evaluate and maintain issue lifecycle discipline while synchronizing each issue with an Orbit task.

It ensures:

- The issue is clearly defined
- The implementation aligns with issue intent
- Status fields reflect reality
- Risks and assumptions are documented
- Next actions are explicit
- Lifecycle state is disciplined
- No duplicate issue records or duplicate Orbit issue tasks are created

This skill does not implement product changes. It performs governance/tracking updates.

---

## Orbit Task Contract

Create and manage Orbit tasks directly with `orbit task` commands.

Required task contract:

- Every tracked issue must have one Orbit task with `--type issue`.
- Task title should match the issue title closely.
- Task `--context` should include relevant files and issue markdown path when available.
- Task `--workspace` should be set to the repository path when available.

---

## Expected Issue Structure

Issues are stored as single markdown files organized by lifecycle state:

```
.orbit/
  agents/<repo_name>/
    issues/
      pending/
        2026-02-14-add-remote-persistence.md
      resolved/
        2026-02-10-fix-cluster-timeout.md
```

Each issue is a single `.md` file.

Required sections inside each issue file:

- Title
- Problem Statement
- Scope
- Constraints (if any)
- Acceptance Criteria
- Lifecycle State (must match parent folder: `pending` or `resolved`)
- Orbit Task ID (required once task is created)
- Notes / Execution Summary (optional)
- Audit (optional)

---

## Tracking Rules

- Ensure no pre-existing pending issue already covers the same concern.
- Ensure no existing open Orbit task of type `issue` already covers the same concern.
- Clearly separate facts from interpretations.
- Flag missing artifacts.
- Explicitly call out stale or ambiguous issues.
- Enforce structured lifecycle discipline.
- Keep issue markdown state and Orbit task state synchronized.

---

## Lifecycle Mapping

Issue lifecycle is determined strictly by folder location:

- `pending/` -> issue is open
- `resolved/` -> issue is resolved

No additional lifecycle states are allowed.

Orbit task status mapping:

- `pending/` -> task status should be `todo`, `in-progress`, or `blocked`
- `resolved/` -> task status should be `done` (closed)

If the issue file contains a "Lifecycle State" field, it must match its parent folder.

---

## Orbit Task Commands

### Search for duplicates

```bash
orbit task search "<issue keywords>" --json
```

Use results to avoid creating duplicate issue tasks.

### Create issue task

```bash
orbit task add \
  --title "<issue title>" \
  --description "<problem summary>" \
  --instructions "<acceptance criteria and next actions>" \
  --context "<file1,file2,.orbit/agents/<repo>/issues/pending/<issue>.md>" \
  --workspace "<repo path>" \
  --type issue \
  --priority <low|medium|high|critical>
```

Record the returned task ID in issue markdown (`Orbit Task ID`).

### Update issue task

```bash
orbit task update <TASK_ID> \
  --description "<updated summary>" \
  --instructions "<updated criteria, notes, risks, next actions>" \
  --status <todo|in-progress|blocked|done|cancelled> \
  --priority <low|medium|high|critical> \
  --context "<updated file list>"
```

### Close issue task

```bash
orbit task close <TASK_ID>
orbit task show <TASK_ID>
```

---

## Standard Workflow

### 1) Search for Existing Records

1. Search pending issue markdown files for overlap.
2. Run `orbit task search "<query>" --json` for overlap.
3. If a duplicate exists, update existing records instead of creating new ones.

### 2) Create Tracking Records

When a new issue is confirmed:

1. Create issue markdown under `pending/`.
2. Run `orbit task add ... --type issue ...`.
3. Record returned task ID in issue markdown under `Orbit Task ID`.

### 3) Update Tracking Records

When issue details change:

1. Update issue markdown sections.
2. Run `orbit task update <TASK_ID> ...`.
3. Confirm issue markdown and task state remain aligned.

### 4) Resolve Issue

When acceptance criteria are met:

1. Move issue markdown from `pending/` to `resolved/`.
2. Run `orbit task close <TASK_ID>`.
3. Run `orbit task show <TASK_ID>` to verify `done`.
4. Include final notes in issue markdown.

---

## Tracking Report Template

```markdown
# Issue Tracking Report - <Issue Name>

## 1. Issue Summary
Concise description of the problem and intended outcome.

## 2. Current Lifecycle State
Issue State: <pending|resolved>
Orbit Task ID: <task-id>
Orbit Task Status: <todo|in-progress|blocked|done|cancelled>
Justification:

## 3. Alignment with Acceptance Criteria
- Criterion:
  - Status: Met / Partially Met / Not Met
  - Notes:

## 4. Gaps or Inconsistencies
- Gap:
  - Impact:

## 5. Risks / Assumptions
- Risk:
  - Severity: Low / Medium / High
  - Mitigation:

## 6. Recommended Next Actions
- Concrete step(s)

## 7. Overall Health Assessment
Healthy / At Risk / Off Track
Rationale:
```

---

## Completion Standard

Tracking is complete when:

- No duplicate issue markdown or Orbit issue task exists for the same concern.
- Lifecycle state is explicit and correct.
- Acceptance criteria are evaluated.
- Gaps are documented.
- Risks are surfaced.
- Clear next actions are defined.
- Tracking report is stored as:

```
~/.orbit/agents/<repo_name>/issues/<pending|resolved>/YYYY-MM-DD-<title>.md
```

- Lifecycle transitions also move the file to the correct folder.
- Linked Orbit task is synchronized and uses type `issue`.

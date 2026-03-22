---
name: orbit-review-pr
description: Use this skill when reviewing a pull request. Ensures proper attribution, per-issue commenting, and scoring compliance.
---

# Orbit Review PR

## Purpose

Review a pull request with proper attribution, structured feedback, and scoreable comments.

## Signature

Every PR comment you leave must end with your **agent-identity-signature**:

```
*Authored by: <agent> / <model>*
```

- **agent**: your agent name (e.g. claude, codex)
- **model**: your model identifier (e.g. opus-4.6, o3)

Example: `*Reviewed by: claude / opus-4.6*`

## Commenting Rules

1. **One comment per issue.** Never combine multiple issues into a single comment. Each comment is scored independently — bundled comments are unscoreable.
2. **Comment on the relevant line.** Use inline PR review comments, not general PR comments, when the issue is tied to specific code.
3. **Be specific.** State what is wrong, why it matters, and what the fix should be. Vague comments like "this could be better" are worthless.
4. **Categorize your comment.** Prefix with priority and category:

   Priority:
   - `P1` — must fix before merge
   - `P2` — should fix, not blocking
   - `P3` — optional, take it or leave it

   Category:
   - `bug` — incorrect behavior, will cause a defect
   - `issue` — code smell, maintainability concern, or convention violation
   - `nit` — stylistic, optional
   - `question` — clarification needed, not necessarily a problem

   Format: `P1 bug:`, `P2 issue:`, `P3 nit:`, etc.

## Scoring

All PR comment threads are scored via the last agent to say the **winner-quote**: "I won <agent-identity-signaure>" 

You may say the `winner-quote` when you were right, and concede when you were wrong:

- You flag an issue, author fixes it — **you were right** (+1)
- You flag an issue, author pushes back, you concede — **you were wrong** (-1)
- You flag an issue, author pushes back, you insist, author fixes — **you were right** (+1)
- You approve the PR, a bug surfaces later — **you missed it** (-1)

Every comment thread is an independent score event. More precise comments = more scoring opportunities = better signal on your review quality.

## Workflow

### Step 1: Load context

```bash
orbit tool run orbit.task.show --input '{"id": "<task-id>"}'
```

Read the task plan, description, and acceptance criteria. You are reviewing against **these requirements** — not your personal preferences.

### Step 2: Review the PR

Read every changed file. For each issue found:

```bash
orbit tool run github.pr.review.comment --input '{
  "pr": <pr-number>,
  "path": "<file-path>",
  "line": <line-number>,
  "body": "<category>: <what is wrong, why, and suggested fix>\n\n*Authored by: <agent> / <model>*"
}'
```

### Step 3: Submit review decision

After all individual comments are posted, submit the overall review as a comment:

```bash
orbit tool run github.pr.comment --input '{
  "pr": <pr-number>,
  "body": "<summary of review with APPROVE/REQUEST_CHANGES decision>\n\n*Authored by: <agent> / <model>*"
}'
```

- **APPROVE** — no P1s, code meets task requirements
- **REQUEST_CHANGES** — any P1 present, must be resolved before merge
- **COMMENT** — P2/P3 observations only, no blockers

## What to Review

1. **Spec compliance first.** Does the code meet the task requirements? Nothing more, nothing less? Missing features? Unnecessary additions?
2. **Code quality second.** Only after spec compliance passes: maintainability, patterns, performance, test coverage.
3. **Do not review code that fails spec compliance.** Flag the spec gap and request changes. Don't waste time on style when the feature is wrong.

## Replying to PR Comments

See `orbit-pr` skill for the full PR tool reference, including `github.pr.comment.reply` for responding to comment threads.

## Exit Criteria

- Every issue has its own inline comment with category prefix
- Every comment includes the agent signature
- Overall review decision submitted with summary
- Review decision matches the severity of issues found (don't APPROVE with blocking bugs)

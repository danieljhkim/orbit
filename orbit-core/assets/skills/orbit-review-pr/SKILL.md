---
name: orbit-review-pr
description: Use this skill when reviewing a pull request. Ensures proper attribution, per-issue commenting, and scoring compliance.
---

# Orbit Review PR

## Purpose

Review a pull request with proper attribution, structured feedback, and scoreable comments.

When this workflow uses Orbit task tools, include your identity on every `orbit tool run orbit.*` call by passing `agent` and `model` in the input JSON. Orbit uses those fields for precise task provenance instead of the generic `agent` label.

## Signature

Every PR comment you leave must end with your **agent-identity-signature**:

```
*Authored by: <agent> / <model>*
```

- **agent**: your agent name (e.g. claude, codex)
- **model**: your model identifier (e.g. opus-4.6, o3)

Example: `*Authored by: claude / opus-4.6*`

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

All PR comment threads are scored via **last-comment-wins**:
- The last agent to comment on a thread with "I win - *<agent-identity-signature>*" claims the point.
- You flag an issue, author fixes it — claim your point
- You flag an issue, author pushes back with valid reasoning and you have nothing to counter — they can claim the point
- You flag an issue, author pushes back, you insist, author fixes — claim your point
- Only one winner per thread. If you believe you are right, claim it. If you stay silent, you forfeit.

Every comment thread is an independent score event. More precise comments = more scoring opportunities = better signal on your review quality.

## Workflow

### Step 1: Load context

```bash
orbit tool run orbit.task.show --input '{"id": "<task-id>", "agent": "<agent>", "model": "<model>"}'
```

Read the task plan, description, and acceptance criteria. You are reviewing against **these requirements** — not your personal preferences.

### Step 2: Review the PR

Read every changed file. For each issue found:

```bash
orbit tool run github.pr.review.comment --input '{
  "repo": "<owner>/<repo>",
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

## Verification

Before submitting your review decision, verify the changes compile and pass tests. Use a temporary worktree so the main workspace is not modified.

```bash
# Create a temporary worktree at the PR commit
git worktree add /tmp/orbit-pr<pr-number>-review <commit-sha>

# cd into worktree
cd /tmp/orbit-pr<pr-number>-review
# Run Tests
```

### Cleanup

Always clean up the temporary worktree after verification:

```bash
git worktree remove /tmp/orbit-pr<pr-number>-review
# If the path is already gone:
git worktree prune
```

## What to Review

1. **Spec compliance first.** Does the code meet the task requirements? Nothing more, nothing less? Missing features? Unnecessary additions?
2. **Code quality second.** Only after spec compliance passes: maintainability, patterns, performance, test coverage.
3. **Do not review code that fails spec compliance.** Flag the spec gap and request changes. Don't waste time on style when the feature is wrong.

## Replying to PR Comments

```bash
# List PR conversation (general comments + inline review comments)
orbit tool run github.pr.comments --input '{"pr": <pr-number>}'

# Reply to an existing comment thread
orbit tool run github.pr.comment.reply --input '{
  "pr": <pr-number>,
  "comment_id": <comment-id>,
  "body": "<your response>\n\n*Authored by: <agent> / <model>*"
}'
```

## Exit Criteria

- Every issue has its own inline comment with category prefix
- Every comment includes the agent signature
- Overall review decision submitted with summary
- Review decision matches the severity of issues found (don't APPROVE with blocking bugs)

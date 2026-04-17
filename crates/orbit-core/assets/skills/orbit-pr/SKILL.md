---
name: orbit-pr
description: Use this skill when creating or reviewing pull requests, replying to PR comments, or working with PR tools.
---

# Orbit PR

## Purpose

Standardize how agents create, review, and discuss pull requests.

Agent signature is auto-appended to all PR bodies and comments by the tools. Do not add it manually.

## PR Tool Reference

All PR interactions go through `orbit tool run`. **Never use `gh api` or `gh pr` directly.**

```bash
# Create a PR
orbit tool run github.pr.create --input '{
  "title": "<short title under 70 chars> [task_id]",
  "body": "<PR body>",
  "head": "<branch>",
  "base": "<target branch>"
}'

# View a PR
orbit tool run github.pr.view --input '{"pr": <pr-number>}'

# List PR conversation (general comments + inline review comments)
orbit tool run github.pr.comments --input '{"pr": <pr-number>}'

# Leave a general PR comment
orbit tool run github.pr.comment --input '{
  "pr": <pr-number>,
  "body": "<summary or non-blocking note>"
}'

# Leave an inline review comment on a specific line
orbit tool run github.pr.review.comment --input '{
  "repo": "<owner>/<repo>",
  "pr": <pr-number>,
  "path": "<file-path>",
  "line": <line-number>,
  "body": "<category>: <what is wrong, why, and suggested fix>"
}'

# Submit a formal review decision
orbit tool run github.pr.review --input '{
  "repo": "<owner>/<repo>",
  "pr": <pr-number>,
  "action": "approve|request-changes",
  "body": "<summary of review>"
}'

# Reply to an existing comment thread
orbit tool run github.pr.comment.reply --input '{
  "repo": "<owner>/<repo>",
  "pr": <pr-number>,
  "comment_id": <comment-id>,
  "body": "<your response>"
}'
```

## Creating a PR

1. **Title** — under 70 characters, summarize the change. Use prefixes: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
2. **Body** — include:
   - Summary of what changed and why
   - Link to the Orbit task ID if applicable
   - Test plan or verification steps
3. **Branch** — use `orbit/<task-id>` naming when tied to a task.
4. **Base** — target the repo's main branch (typically `main`).

## Reviewing a PR

### What to review

1. **Spec compliance first.** Does the code meet the task requirements? Nothing more, nothing less? Missing features? Unnecessary additions?
2. **Code quality second.** Only after spec compliance passes: maintainability, patterns, performance, and validation coverage consistent with repo instructions.
3. **Do not review code that fails spec compliance.** Flag the spec gap and request changes.

### Load context

```bash
orbit tool run orbit.task.show --input '{"id": "<task-id>"}'
```

Read the task plan, description, and acceptance criteria. Review against **these requirements** — not personal preferences.

### Comment rules

1. **One comment per issue.** Never combine multiple issues into a single comment. Bundled comments are unscoreable.
2. **Comment on the relevant line.** Use inline PR review comments when the issue is tied to specific code.
3. **Be specific.** State what is wrong, why it matters, and what the fix should be.
4. **Categorize your comment.** Prefix with priority and category:

   Priority: `P1` (must fix), `P2` (should fix), `P3` (optional)
   Category: `bug`, `issue`, `nit`, `question`
   Format: `P1 bug:`, `P2 issue:`, `P3 nit:`, etc.

### Review decision

- **approve** — no P1s, code meets task requirements
- **request-changes** — any P1 present, must be resolved before merge
- For non-blocking notes only, leave comments or threads without escalating to a blocking review.

### Verification

Before submitting your review decision, verify the change with repo-approved commands. If the repo forbids tests, do not add or run them just for review.

```bash
git worktree add /tmp/orbit-pr<pr-number>-review <commit-sha>
cd /tmp/orbit-pr<pr-number>-review
# Run the allowed verification commands for this repo
git worktree remove /tmp/orbit-pr<pr-number>-review
```

## Replying to PR Comments

- **One reply per thread.** Address the specific point raised.
- Whether you are the reviewer or the implementer, the same rules apply.

## Scoreboard

Orbit currently tracks PR review comment sync counts and merge-with/without-revision counts. It does not automatically adjudicate per-thread winners, so do not rely on slogan-based "thread winning" claims.

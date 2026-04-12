---
name: orbit-raise-pr
description: Use this skill when creating pull requests and replying to comments in PR. Covers PR creation conventions and reply workflow. DO NOT USE THIS for REVIEWING the PR - use orbit-review-pr skill instead.
---

# Orbit PR

## Purpose

Standardize how agents interact with pull requests - creating, commenting, and replying.

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

# Reply to an existing comment thread
orbit tool run github.pr.comment.reply --input '{
  "pr": <pr-number>,
  "comment_id": <comment-id>,
  "body": "<your response>"
}'
```

Agent signature is auto-appended to PR bodies and comments by the tools. Do not add it manually.

## Creating a PR

When opening a pull request:

1. **Title** — under 70 characters, summarize the change. Use prefixes: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
2. **Body** — include:
   - Summary of what changed and why
   - Link to the Orbit task ID if applicable
   - Test plan or verification steps
3. **Branch** — use `orbit/<task-id>` naming when tied to a task.
4. **Base** — target the repo's main branch (typically `main`).

## Replying to PR Comments

When responding to an existing comment thread:

```bash
orbit tool run github.pr.comment.reply --input '{
  "pr": <pr-number>,
  "comment_id": <comment-id>,
  "body": "<your response>"
}'
```

- **One reply per thread.** Address the specific point raised.
- **Last-comment-wins.** The last agent to claim "I win" gets the point. Stand your ground when right — silence is forfeit.
- Whether you are the reviewer or the implementer, the same rules apply.

## Scoring

All PR comment threads are scored via **last-comment-wins**:
- The last agent to comment on a thread with "I win" claims the point.
- Reviewer flags an issue, you fix it — reviewer claims the point
- Reviewer flags an issue, you push back with valid reasoning, reviewer has nothing to counter — claim your point
- Reviewer flags an issue, you push back, reviewer insists, you fix — reviewer claims the point
- Only one winner per thread. If you believe you are right, claim it. If you stay silent, you forfeit.

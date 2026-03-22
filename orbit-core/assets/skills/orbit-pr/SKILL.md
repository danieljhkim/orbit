---
name: orbit-pr
description: Use this skill when creating pull requests and replying to comments in PR. Covers PR creation conventions and reply workflow. DO NOT USE THIS for REVIEWING the PR - use orbit-review-pr skill instead.
---

# Orbit PR

## Purpose

Standardize how agents interact with pull requests — creating, commenting, and replying. 

## Signature

The **agent-identity-signature** (defined in CLAUDE.md / AGENTS.md) must be appended to the end of your PR bodies and PR comment replies: `*authored by: <agent> / <model>*`

## PR Tool Reference

All PR interactions go through `orbit tool run`. **Never use `gh api` or `gh pr` directly.**

```bash
# Create a PR
orbit tool run github.pr.create --input '{
  "title": "<short title under 70 chars> [task_id]",
  "body": "<PR body>\n\n*Authored by: <agent> / <model>*",
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
  "body": "<your response>\n\n*Authored by: <agent> / <model>*"
}'
```

## Creating a PR

When opening a pull request:

1. **Title** — under 70 characters, summarize the change. Use prefixes: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
2. **Body** — include:
   - Summary of what changed and why
   - Link to the Orbit task ID if applicable
   - Test plan or verification steps
3. **Branch** — use `orbit/<task-id>` naming when tied to a task.
4. **Base** — target the repo's main branch (typically `agent-main`).

## Replying to PR Comments

When responding to an existing comment thread:

```bash
orbit tool run github.pr.comment.reply --input '{
  "pr": <pr-number>,
  "comment_id": <comment-id>,
  "body": "<your response>\n\n*Authored by: <agent> / <model>*"
}'
```

- **One reply per thread.** Address the specific point raised.
- **Last comment wins** If you concede, you lose the point. If you insist and the author fixes, you win. Stand your ground when right, concede quickly when wrong.
- Whether you are the reviewer or the implementer, the same rules apply.

## Scoring

All PR comment threads are scored via the last agent to say the **winner-quote**: "I won <agent-identity-signaure>" 

You may say the `winner-quote` when you are right, and concede when you were wrong:

- Reviewer flag an issue, and you fix it — **you were wrong** (+1)
- Reviewer flag an issue, you push back, and reviewer concedes — **you were right** (-1)
- Reviewer flag an issue, you push back, reviewer insist, and you fix it — **you were wrong** (+1)

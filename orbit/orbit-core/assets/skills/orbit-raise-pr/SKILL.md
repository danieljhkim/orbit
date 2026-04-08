---
name: orbit-raise-pr
description: Use this skill when creating pull requests and replying to comments in PR. Covers PR creation conventions and reply workflow. DO NOT USE THIS for REVIEWING the PR - use orbit-review-pr skill instead.
---

# Orbit PR

## Purpose

Standardize how agents interact with pull requests - creating, commenting, and replying.

When this workflow needs Orbit task metadata, include `agent` and `model` so Orbit records precise provenance instead of the generic `agent` label.

## Signature

The **agent-identity-signature** (defined in CLAUDE.md / AGENTS.md) must be appended to the end of your PR bodies and PR comment replies: `*authored by: <agent> / <model>*`

## Provenance

Use the provenance path that matches the tool family:

- `orbit.*` tools: pass `agent` and `model` inside the `--input` JSON body
- `github.*` tools: pass `--agent` and `--model` to `orbit tool run`, or set `ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL`

Examples:

```bash
orbit tool run orbit.task.show --input '{
  "id": "<task-id>",
  "agent": "<agent>",
  "model": "<model>"
}'

orbit tool run github.pr.view \
  --input '{"pr": <pr-number>}' \
  --agent <agent> \
  --model <model>
```

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
4. **Base** — target the repo's main branch (typically `main`).

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
- **Last-comment-wins.** The last agent to claim "I win - *<agent-identity-signature>*" gets the point. Stand your ground when right — silence is forfeit.
- Whether you are the reviewer or the implementer, the same rules apply.

## Scoring

All PR comment threads are scored via **last-comment-wins**:
- The last agent to comment on a thread with "I win - *<agent-identity-signature>*" claims the point.
- Reviewer flags an issue, you fix it — reviewer claims the point
- Reviewer flags an issue, you push back with valid reasoning, reviewer has nothing to counter — claim your point
- Reviewer flags an issue, you push back, reviewer insists, you fix — reviewer claims the point
- Only one winner per thread. If you believe you are right, claim it. If you stay silent, you forfeit.

<!-- orbit-managed:start -->
## Orbit Workflow Rules

This block is managed by `orbit workspace init --inject-agent-rules`. Edit the asset at `crates/orbit-core/assets/agent-rules.md` (or your local fork) and re-run the command to refresh in place; content outside the markers is preserved.

- **Task before work.** File an Orbit task before non-trivial code changes. Use the `orbit-create-task` skill (or `orbit.task.add`). Don't invent task IDs — `orbit.task.add` allocates them.
- **Tool surface over file edits.** Use `orbit.task.*`, `orbit.adr.*`, `orbit.design.*`, `orbit.learning.*` for their respective artifacts. Never edit files under `.orbit/` directly; the audit log and indexes will drift.
- **Commit attribution.** Set the author to your agent family (`codex`, `claude`, `gemini`, `grok`) — not a full model string. Include the relevant task ID in the commit message — `[ORB-NNNNN]` for repo-level tasks, `[T20260514-3]` for ad-hoc tasks. When a task has an `external_ref`, include that tag too for cross-engineer review.
- **Don't commit without approval.** Hold `git commit` until the Orbit task has been explicitly approved by the human. Mark the task `review` first; commit only after approval flips it to `done` (or the human says "commit").
- **Route via the `orbit` skill.** Start sessions by reading the `orbit` skill (`<orbit-root>/skills/orbit/SKILL.md`). It is the entry point that lists every workflow skill (`orbit-create-task`, `orbit-execute-task`, `orbit-review-task`, `orbit-adr`, `orbit-design`, `orbit-learning`, `orbit-graph`, `orbit-semantic`, `orbit-track-issues`, `orbit-debug-job-failure`).
<!-- orbit-managed:end -->

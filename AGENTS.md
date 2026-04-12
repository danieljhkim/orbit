# AGENTS.md

Project instructions for agents working on Orbit.

## Project Don'ts

- Don't commit until the Orbit task has been explicitly approved by the human.
- Don't invent task IDs — get them from `orbit.task.add`.
- Don't edit task files directly — use `orbit.task.update`.
- Don't add cross-crate dependencies without checking the architecture diagram below.
- When you hit friction, ambiguity, naming drift, or duplicated sources of truth: file a self-reported friction task via the `orbit-track-issues` skill instead of working around it.
- Reserve task type `friction` for agent self-reports only. Do not use `friction` for normal user-requested work, backlog shaping, or generic bug tracking.

## Project Do's

- Use subagents to support you through large tasks and keep your context window clean.
- Use terse/succinct prose in all agent-written text: tasks, docs, comments, commit messages.

## Build / Test / Lint

- Build: `make build`
- Test:  `make test`
- Fmt:   `make fmt`

All must pass before a task moves to `review`.

## Crate Architecture

```
orbit-types → orbit-policy, orbit-exec, orbit-knowledge → orbit-tools → orbit-store, orbit-agent → orbit-engine → orbit-core → orbit-cli
```

- **orbit-types**: leaf — no internal deps. Shared types, `OrbitError`, ID generation.
- **orbit-knowledge**: knowledge/graph parsing and storage helpers. Depends on `orbit-types`; consumed by `orbit-tools` and `orbit-cli`.
- **orbit-store**: layered store pattern (YAML + SQLite). Match existing modules when adding new ones.
- **orbit-agent**: `AgentRuntime` trait. New agent families go here, not elsewhere.
- **orbit-engine**: activity/job execution, template rendering, retry logic.
- **orbit-core**: runtime bootstrap, config layering, command dispatch, default asset seeding.
- **orbit-cli**: clap-based CLI entry point.

## Scoping Rules

| Artifact        | Strategy           | Rationale                                        |
|-----------------|--------------------|--------------------------------------------------|
| Tasks           | WorkspaceOnly      | Per-repo backlog, no cross-project leaking       |
| Activities/Jobs | MergeByKey         | Global defaults + workspace overrides            |
| Job Runs        | WorkspaceOnly      | Execution artifacts are workspace-local          |
| Skills          | WorkspaceReplaces  | Workspace has full control over available skills |
| Audit           | GlobalOnly         | Single authoritative event trail                 |

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router.

## Task Authoring Quality

Follow the `## Task Quality Standards` section in `orbit-create-task` skill: deterministic mock-based testing, explicit observable definitions for summary fields (`purpose`, etc.), and testability-preserving implementation patterns.

## Agent Identity

**Signature** (used in PR bodies, comments, commit messages, review summaries):

> *authored by: codex / gpt-5.4*

**Commits**:

- Use the agent commit identity (e.g. `codex`) as author/committer when the agent made the change.
- Include the task ID in the commit message when the commit is associated with an Orbit task (e.g. `[T20260320-001234]`).

## Scoreboards

- `.orbit/scoreboard/pr.json` — PR merge rates, revision counts, review comment validity per agent/model.
- `.orbit/scoreboard/friction_bounty.json` — self-reported agent friction reports (issues-reported, issues-accepted, issues-rejected) per agent/model. Rejected reports count against the reporter; quality over quantity.

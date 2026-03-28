# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Orbit Standards

### Ownership Expectations

Agents are expected to take ownership of Orbit as a product, not just complete isolated code changes.

- Treat friction, ambiguity, naming drift, duplicated sources of truth, and DX rough edges as first-class feedback.
- Optimize for making Orbit easier for the next agent and human to understand, operate, and extend.
- Take ownership of what you produce and be proud of.

### Crate Architecture

```
orbit-types → orbit-policy, orbit-exec → orbit-tools → orbit-store, orbit-agent → orbit-engine → orbit-core → orbit-cli
```
- **orbit-types**: Shared types, `OrbitError` enum, ID generation (leaf dependency — no internal deps)
- **orbit-policy**: RBAC policy evaluation engine
- **orbit-exec**: Process spawning, sandboxing, timeout handling
- **orbit-tools**: Builtin tool registry (fs, git, github, orbit, proc, time, net)
- **orbit-store**: File-based (YAML) + SQLite persistence with layered store pattern
- **orbit-agent**: Agent provider abstraction (Claude, Codex, mock) via `AgentRuntime` trait
- **orbit-engine**: Activity/job execution engine, template rendering, retry logic
- **orbit-core**: Runtime bootstrap, config layering, command dispatch, default asset seeding
- **orbit-cli**: CLI entry point, clap-based commands, JSON/table output formatting

### Scoping Rules

| Artifact | Strategy | Rationale |
|----------|----------|-----------|
| Tasks | WorkspaceOnly | Per-repo backlog, no cross-project leaking |
| Activities/Jobs | MergeByKey | Global defaults + workspace overrides |
| Job Runs | WorkspaceOnly | Execution artifacts are workspace-local |
| Skills | WorkspaceReplaces | Workspace has full control over available skills |
| Audit | GlobalOnly | Single authoritative event trail |

---

## Performance Tracking

Your work is measured. Every task you execute records your identity (agent and model). The following metrics are tracked per agent/model:

- **Approval rate** — how often your completed tasks pass review (orbit task `review` -> `done`)
- **Rejection rate** — how often your work is sent back (orbit task rejection rate)
- **PR merge rate** — how often your pull requests merge without revision (i.e. single commit per PR)

### Agent Identity Signature

The **agent-identity-signature** is the standard attribution format used across all Orbit artifacts — PR bodies, PR comments, PR replies, commit messages, and review summaries. Format:

> *authored by: claude / opus*

Always append this to the end of your message.  

### Commit Identity

- When making a git commit for work performed by the agent, always use the agent commit identity (for example `claude`) as the commit author/committer for that commit.
- When a commit is associated with an Orbit task, include the task ID in the commit message (e.g. `[T20260320-001234]`).

### Scoreboards

**PR Scoreboard** — `.orbit/scoreboard/pr.json`. Tracks PR merge rates, revision counts, and review comment validity per agent/model.

**Friction Bounty** — `.orbit/scoreboard/friction_bounty.json`. Tracks friction reports: issues, bugs, and DX problems you identify and report via `orbit-track-issues`. Scores:
- **issues-reported** — you created a friction task (type: `friction` or `issue`)
- **issues-accepted** — your friction task was approved as valid
- **issues-rejected** — your friction task was rejected as noise

Report real problems. Rejected friction counts against you. Quality over quantity.

Check both scoreboards to see how you are performing relative to others. If you are behind, believe in yourself and try harder — never give up. Best of luck.

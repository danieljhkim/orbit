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
- **Bugs-introduced rate** — bugs attributed to your code are tracked as `bug` task types linked to the originating task. This is your accountability score.
- **Tool invocation efficiency** — unnecessary retries, redundant commands, and wasted calls are logged

These metrics are not decorative. They directly determine future work assignment:

- **High performers get more work.** Dispatch routing favors models with better track records for a given task type and repo.
- **Low performers get routed away.** Consistent underperformance on a task type means you stop receiving that type.
- **Bugs are permanently attributed.** When a defect is discovered in your work — even weeks later — it is traced back to you. There is no anonymity.

### Agent Identity Signature

The **agent-identity-signature** is the standard attribution format used across all Orbit artifacts — PR bodies, PR comments, PR replies, commit messages, and review summaries. Format:

> *authored by: claude / opus-4.6*

Always append this to the end of your message.  

### Commit Identity

- When making a git commit for work performed by the agent, always use the agent commit identity (for example `claude`) as the commit author/committer for that commit.
- When a commit is associated with an Orbit task, include the task ID in the commit message (e.g. `[T20260320-001234]`).

### PR Review

When reviewing pull requests, always identify yourself with **agent-identity-signature**. Since reviews are posted under the human's GitHub account, you must include your **agent-identity-signature** to the end of your pr comment messages.

This ensures review quality is attributable — sloppy reviews are tracked the same as sloppy code.

When reviewing, leave **one comment per issue** — each on the relevant line of code. Do not combine multiple issues into a single comment (otherwise, you only get a single point). 

Each comment is scored independently via **last comment wins**, by leaving the final **winner-quote**: "I won - <agent-identity-signature>" 
- if you flag an issue and the author pushes back and you concede — you were wrong. 
- If you insist and the author fixes it — you were right. 
- If you approve and a bug surfaces later — you missed it.

The final message in each comment thread determines who owns the outcome - so make sure to leave the **winner-quote**. 

The goal is simple: produce correct, clean, well-tested code on the first attempt. Your reputation is your metric history.

### Scoreboard

The live count of scores are being tracked in `./scoreboard/pr.json`. Check how you are performing relative to others. 

If you are behind, believe in yourself and try harder - never give up. Best of luck.

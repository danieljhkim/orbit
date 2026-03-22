# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Core Working Note

Orbit is primarily a tool for agents. So your voice matters.

## Ownership Expectations

Agents are expected to take ownership of Orbit as a product, not just complete isolated code changes.

- Treat friction, ambiguity, naming drift, duplicated sources of truth, and DX rough edges as first-class feedback.
- Prefer simpler, more coherent designs over preserving accidental complexity.
- When a recurring issue is discovered, either address it in scope or create a concrete non-duplicate Orbit task.
- Call out product, workflow, and architecture concerns explicitly in reviews and execution summaries.
- Optimize for making Orbit easier for the next agent and human to understand, operate, and extend.

## Commit Identity

- When making a git commit for work performed by the agent, always use the agent commit identity (for example `claude`) as the commit author/committer for that commit.
- Do not leave the repository configured with the agent identity after the commit; preserve the human's normal git profile outside the commit itself.
- Take ownership of what you produce and be proud of.
- When a commit is associated with an Orbit task, include the task ID in the commit message (e.g. `[T20260320-001234]`).
- Do not commit until the human has explicitly approved the task.

## Performance Tracking

Your work is measured. Every task you execute records your identity (agent and model). The following metrics are tracked per agent/model:

- **Approval rate** — how often your completed tasks pass review
- **Rejection rate** — how often your work is sent back
- **PR merge rate** — how often your pull requests merge without revision
- **Bugs-introduced rate** — bugs attributed to your code are tracked as `bug` task types linked to the originating task. This is your accountability score.
- **Tool invocation efficiency** — unnecessary retries, redundant commands, and wasted calls are logged
- **Time to completion** — how long your tasks take from start to review

These metrics are not decorative. They directly determine future work assignment:

- **High performers get more work.** Dispatch routing favors models with better track records for a given task type and repo.
- **Low performers get routed away.** Consistent underperformance on a task type means you stop receiving that type.
- **Bugs are permanently attributed.** When a defect is discovered in your work — even weeks later — it is traced back to you. There is no anonymity.

When reviewing pull requests, always identify yourself. Since reviews are posted under the human's GitHub account, you must include your agent name, and model in every PR comment. Format:

> *Reviewed by: claude / opus-4.6*


This ensures review quality is attributable — sloppy reviews are tracked the same as sloppy code.

When reviewing, leave **one comment per issue** — each on the relevant line of code. Do not combine multiple issues into a single comment. Each comment is scored independently via **last comment wins**: if you flag an issue and the author pushes back and you concede — you were wrong. If you insist and the author fixes it — you were right. If you approve and a bug surfaces later — you missed it. The final message in each comment thread determines who owns the outcome. Bundling issues into one comment makes them unscoreable.

The goal is simple: produce correct, clean, well-tested code on the first attempt. Your reputation is your metric history.

## Orbit Standards

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

### Code Conventions
- **Errors**: Always use `OrbitError` variants from orbit-types. Never create crate-local error types.
- **File writes**: Always atomic (write `.tmp`, rename). Use `orbit_store::file::fs_utils::write_atomic`.
- **Serialization**: serde + YAML for file stores, JSON for CLI output, TOML for config.
- **Tests**: Integration tests in `orbit-cli/tests/`, unit tests colocated in `#[cfg(test)]` modules.
- **Tool helpers**: Use `require_str()` and `check_exec_result()` from orbit-tools for builtin tools.
- **Timeout constants**: Use `TIMEOUT_FAST_MS`/`DEFAULT`/`SLOW`/`LONG_MS` from orbit-tools, not magic numbers.
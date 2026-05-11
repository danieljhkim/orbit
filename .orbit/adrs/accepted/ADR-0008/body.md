## Context
Per-run worktrees are supposed to isolate task implementation, but `backend: cli` children previously inherited the pipeline worker cwd and only learned the intended workspace through prompt/input data.

## Decision
Resolve CLI subprocess cwd before spawn from `input.workspace_path`, then task snapshot `workspace_path`, then best-effort `ToolContext.workspace_root`. Declared input/task paths fail fast if stale, and the selected cwd is recorded in the CLI started audit event plus line-level tracing.

## Consequences
- The runtime, not the prompt, controls where relative paths in provider CLIs resolve.
- Groundhog and CLI dispatch share one workspace resolver, reducing future drift between orchestration and implementation attempts.
- Cost: stale declared worktrees now fail before spawn instead of silently running from the parent process directory.

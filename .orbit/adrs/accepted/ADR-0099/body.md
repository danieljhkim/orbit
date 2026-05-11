## Context
Naive subprocess code on Unix leaves orphan grandchildren holding open pipe write ends, which causes the parent's `wait_with_output` to hang when the orphan never exits. Earlier versions of orbit-exec hit this exact failure when an agent's tool spawned long-lived helpers.

## Decision
On Unix, every spawned child calls `command.process_group(0)` so the child becomes a process-group leader (PGID = PID). The supervision layer kills the entire group via `killpg` when the child exits, when the parent receives SIGINT/SIGTERM, or when the deadline expires.

## Consequences
- Orphan subprocesses are reaped, and signal handling can target the whole tree with one syscall.
- Cost: tools that intentionally fork detached helpers (e.g., long-running daemons) cannot do so under orbit-exec without explicitly creating their own process group inside the child.

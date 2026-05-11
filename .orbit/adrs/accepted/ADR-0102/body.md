## Context
The `Sandbox` trait is the seam where kernel-level or container-level isolation would attach to `orbit-exec`. The trait shipped with the supervision rework, but no real impl is registered.

## Decision
Ship `NoSandbox` as the default and only implementation. Defer kernel-level isolation (bubblewrap, sandbox-exec, container, seccomp) until policy enforcement at the tool layer is judged insufficient and the platform-coverage cost is understood. The trait surface is stable so a future impl can attach without changing the runner.

## Consequences
- The trait surface is stable for future isolation, while today's generic runner stays explicit about relying on tool-layer policy.
- Cost: a tool that performs fs work without `enforce_fs_policy` (or a future non-builtin tool) has no exec-level isolation backstop. This is the structural reason §1.1 of [3_vision.md](./3_vision.md) lists real sandboxing as the top open question.

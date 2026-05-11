## Context
A timed-out or interrupted child needs a chance to flush state before being killed, but the supervisor cannot wait indefinitely. The escalation policy needs a single, predictable shape.

## Decision
`terminate_process_group` sends `SIGTERM` (or the supplied signal) to the group, polls `process_group_is_alive` for `TERMINATION_GRACE_PERIOD = 5 seconds`, and on expiry sends `SIGKILL` to the group plus a direct `child.kill()`/`child.wait()`. stderr is annotated with `process timed out` (deadline path) or `process interrupted by signal SIG…` (parent-signal path).

## Consequences
- Termination is deterministic, and annotated stderr distinguishes timeout, signal, and clean-exit paths.
- Cost: the 5-second constant is global. Activities that need a longer drain (database flush, large I/O cleanup) cannot extend it without code changes.

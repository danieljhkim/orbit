## Context
Review-thread reminders need a cheap way to know which task owns the current agent turn. Inferring from cwd or scanning task files would make every PreToolUse call depend on filesystem heuristics, while the engine already knows the executing task when it seeds ORBIT_TASK_ID.

## Decision
The hook treats ORBIT_ACTIVE_TASK_ID as the explicit active-task binding, with ORBIT_TASK_ID as a compatibility fallback for existing execution paths. Orbit execution code seeds both values when the activity input contains a task id, and hook state is still scoped by the existing session id plus parent-pid state-file key.

## Consequences
- Review-thread surfacing remains a local task-store read and does not perform network I/O or cwd inference.
- Existing ORBIT_TASK_ID-spawned executions keep working while newer shims can depend on the clearer ORBIT_ACTIVE_TASK_ID name.
- Cost: Orbit now has two task-id environment names during a compatibility window, so documentation and tests must keep their precedence explicit.
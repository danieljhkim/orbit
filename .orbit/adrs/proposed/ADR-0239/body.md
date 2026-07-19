## Context

Ship-pipeline conflict avoidance currently depends on manually declared `context_files`. Declaration is a guess made at authoring time, is frequently omitted (e.g. ORB-10316/ORB-10317 shipped 2026-07-19 with overlapping `learning_hook.rs` scope and no declarations), and imposes upfront planning cost on every task whether or not a conflict would ever occur. Merge conflicts are rare but expensive; upfront declaration is cheap per-file but paid universally. The optimal balance is to pay planning cost only where the author already knows a file is hot, and otherwise learn the real footprint at runtime.

Alternatives considered and rejected:

- **Tag-overlap conflict resolution:** tags are not reliably feature-scoped — they represent many orthogonal things — so overlap is neither necessary nor sufficient for conflict. Rejected.
- **Write-time file locking (PreToolUse deny):** requires a denial policy, invites deadlock or stalled agent loops, and interrupts in-flight runs. The same observed-footprint data applied at scheduling granularity achieves the benefit without arbitration semantics. Rejected.
- **Mandatory upfront context_files:** universal planning cost for rare conflicts. Rejected.

## Decision

1. **`context_files` stays optional** and keeps its current meaning: declared context the agent should read. No upfront requirement.
2. **New sibling field `touched_files`:** during a job-run, the Edit/Write PreToolUse hook auto-appends each written file (repo-relative path, deduplicated) to the owning task's `touched_files`. The two fields stay semantically distinct: declared intent vs observed footprint. Prerequisite plumbing: the run environment must export the owning task id (`ORBIT_TASK_ID`), alongside the `ORBIT_SESSION_ID` propagation fix (ORB-10316).
3. **Ship gating applies to auto mode only.** In auto (backlog-discovery) mode, a candidate is dispatchable only if the union `context_files ∪ touched_files` is disjoint from that union across all in-flight tasks. On overlap the candidate is **deferred, not rejected**: it stays queued and is re-evaluated when an in-flight task reaches a terminal state.
4. **Explicit ship bypasses the gate.** `orbit run ship <id>` (or `workflow_ship` with explicit task ids) dispatches unconditionally — operator intent is authoritative; no overlap check.
5. **Auto mode is refactored to "empty the backlog":** instead of snapshotting initially detected backlog items and shipping only those, the pipeline continuously re-evaluates — picking up newly shippable, newly unblocked, and gate-cleared tasks — until the backlog is empty or nothing is dispatchable.

## Consequences

- Zero upfront planning cost on typical tasks; declaration becomes opt-in insurance for known-hot files. Residual conflicts concentrate on task pairs that both declared nothing and were both dispatched before either footprint grew — an accepted race window. Git merge remains the final backstop; the gate reduces conflicts, it does not eliminate them.
- `touched_files` doubles as an accurate observed footprint: follow-up and rerun tasks inherit better context than hand-maintained declarations, and the field provides ground-truth data for any future footprint analytics.
- Auto-mode drain semantics make scheduled/auto ship runs longer-lived and stateful; the gate's defer queue must be observable (run status should show what is deferred and why) to avoid silent starvation of repeatedly deferred tasks.
- Explicit-ship bypass means an operator can knowingly ship an overlapping task; resulting rebase/merge conflicts on that path are by choice, handled at merge as today.
- Hook write path must fail open: if the touched_files append fails (orbit unreachable, task coupling missing), the Edit/Write proceeds and the failure is logged — footprint gating degrades to today's behavior, never blocks work.
- Cost: one new task field and migration; PreToolUse hook gains a write path to task state (today it only reads/injects); ship pipeline gains a defer queue and drain loop — moderate one-time implementation cost in orbit-cmd and the task_auto_pipeline workflow, plus the ORBIT_TASK_ID env plumbing shared with ORB-10316.
## Context

A `blocked_by` edge is satisfied only when the target reaches `done` (`TaskStatus::satisfies_dependency`). Everything else counted uniformly as "unmet", and dispatch treated unmet as "wait": `reserve_locks` returned `reserved: false`, and `task_gate_pipeline` polled 120 times at 30-second intervals before `gate_starvation_fail` ended the run.

That conflates two different situations. A dependency in `backlog` / `in-progress` / `review` can still reach `done`, so waiting is correct. A dependency that is `archived`, `rejected`, or no longer resolves to any task cannot reach `done` by the passage of time — only an operator editing the task graph can clear it. Observed on ORB-10586, which kept `blocked_by: ORB-10576` after ORB-10576 was archived: the run polled for an hour and then failed with a message reporting `conflicting_files`, which was empty, so the diagnostic named no blocker at all and pointed at the wrong subsystem.

## Decision

Admission classifies every non-satisfying dependency edge as either a wait or a dead end, and refuses dead ends immediately.

`TaskStatus::dependency_dead_end()` returns `Some(DependencyDeadEnd)` for `archived` and `rejected` and `None` for every other status; a dependency ID that resolves to no task is `DependencyDeadEnd::Missing`. `unsatisfiable_task_dependencies()` in `orbit-common` applies this per task, and `reserve_locks` fails the activity before the first poll when the set is non-empty, with a `task.dependencies.unsatisfiable:` message naming each offending task/dependency pair, the dependency's status, and its remedy.

What counts as *satisfied* is unchanged: `Done`-only. This decision makes an unsatisfiable edge fail loudly, and deliberately does not widen admission. `Archived` remains a soft-delete that does not satisfy a dependency.

The waiting path is untouched — a reachable dependency still yields `reserved: false` with `waiting_on_deps`, the same poll cadence, and the same eventual `gate.starvation` timeout. That timeout now also reports `waiting_on_deps`, since it previously named no blocker for a dependency-starved bundle either.

Rejected alternatives:

- **Widen `satisfies_dependency()` to accept `archived`/`rejected`.** Treats a soft-deleted or declined task as delivered work, and would silently ship a task whose stated prerequisite never happened. The edge is stale data; the fix is to report it, not to accept it.
- **Leave enforcement to `gate_starvation_fail` and only improve its message.** Cheaper, but still costs a full wait budget (an hour at seeded defaults) per stale edge and reports the failure as starvation, which it is not.
- **Validate at edge-creation time only.** Does not help: the edge was valid when written and became unsatisfiable later, when the dependency was archived.

## Consequences

- A stale `blocked_by` edge now fails in seconds with the blocking IDs named, instead of after the full poll budget with an empty file-conflict list.
- The failure text is distinguishable by prefix: `task.dependencies.unsatisfiable` means the task graph is wrong; `gate.starvation` means waiting was legitimate but ran out of budget.
- The epic-rollup predicate `is_feature_child_terminal_status` in `backlog_exclusion.rs` still folds `archived` and `review` into `"done"` for orchestration state. That surface is deliberately not converged here — it answers "should this child be dispatched again?", not "is this prerequisite delivered?" — so two different notions of terminal now coexist in the codebase and a reader must not assume either one generalizes.
- **Cost:** dependency semantics now live in two predicates that must stay in sync. `satisfies_dependency()` answers "is this edge closed?" and `dependency_dead_end()` answers "could it ever close?", and they partition `TaskStatus` between them. A future status variant that is added to neither is silently treated as a legitimate wait forever — reproducing exactly the hour-long stall this decision removes, but for a status nobody classified. `dependency_dead_end()` matches variants exhaustively rather than using a wildcard so that adding a status breaks the build instead.
- **Cost:** the change converts a class of slow timeouts into fast hard failures. An operator who was restoring an archived dependency inside the old one-hour poll window now sees the gate run fail before the restore lands, and must re-dispatch.
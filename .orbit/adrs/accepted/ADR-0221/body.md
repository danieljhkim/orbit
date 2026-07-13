## Context
The GC design (gc/2_design.md §9-§10) aspires to have subscriber-init hooks stop deleting log archives once `orbit gc logs` owns retention, deferring all deletion to an explicit apply. But Orbit has no resident daemon and automated `orbit gc logs` is a future task (ORB-10189); removing opportunistic startup pruning now would regress the ORB-00415 disk bound on always-on hosts until that automation lands. The alternative was to make startup rotate-only and let archives accumulate until an operator runs `orbit gc logs --apply`.

## Decision
Keep opportunistic startup pruning, but route both it and `orbit gc logs` through one extracted classifier (`log_rotation::plan_prune`). Startup deletes best-effort as before; the CLI collector plans/applies the same age + total-size policy with reporting, locking, and revalidation. Neither path ever deletes or truncates the active inode.

## Consequences
- One retention policy: startup pruning and `orbit gc logs` can never disagree because they share `plan_prune`; the CLI adds an inspectable, on-demand surface over the same budgets.
- The v1 log GC does not require a scheduler to keep archives bounded; disk safety from ORB-00415 is preserved.
- Divergence from gc/2_design.md §9's "startup hooks do not delete": §3.3/§9/§10 are updated to describe shared-classifier pruning. The §5 active-inode invariant is unchanged and still honored.
- Cost: startup still performs a small delete pass on every subscriber init; when automated `orbit gc logs` lands (ORB-10189), this decision should be revisited so deletion can move fully behind the explicit apply gate.
## Context

Catalog assets and the installed binary are independently versioned artifacts. `pr_failure_handoff` shipped as an activity asset bound to `task_pr_pipeline`'s `failure_activity` ([ADR-0246]) while orbit-core's v2 dispatch table never gained a forwarding arm for it, so the hook answered `deterministic action not registered` on every invocation — jrun-20260725-1620-4, -1642-3, and -1620-10, each after the job had admitted a task, built a worktree, and spent 18–42 minutes implementing and validating it. Nothing was committed, pushed, or published. `worktree_gc` carried the identical gap. A failure hook is the last preservation boundary, so discovering incompatibility *inside* it is the worst possible time. The real alternatives were to version installed assets against the binary (a distribution mechanism for what is really a runtime-capability question, and it cannot see workspace-local catalogs at all), or to make the dispatcher tolerate unknown actions (silently skipping a preservation hook is strictly worse than failing).

## Decision

Make the runtime host report its capability: `V2RuntimeHost::has_deterministic_action(action)` — defaulting to `true` so hosts that cannot enumerate a registry keep surfacing misses at dispatch. `validate_job_deterministic_actions` consults it for every reachable resolved deterministic activity (job- and step-level `recovery_activity`, job `failure_activity`, and every target, recursing through `parallel:`/`fan_out:`/`loop:`) and runs inside `execute_job_with_resume` before the first step, so an unavailable action fails the run ahead of `worktree_setup`'s workflow admission with a `DeterministicActionUnavailable` naming the activity and the action. orbit-core's dispatch table publishes its names as one list that also rejects unlisted actions up front, so the advertised capability cannot exceed what dispatch accepts, and the missing `pr_failure_handoff` / `worktree_gc` arms are registered.

## Consequences


- Catalog/runtime skew is a load-time failure with no task-lifecycle or worktree side effect, instead of a terminal failure that strands completed work.
- A seeded-asset sweep pins the direction that actually broke: every shipped job's reachable actions and every seeded deterministic activity's action must be dispatchable, so a future asset can no longer reference an action the binary lacks without CI failing.
- The failure hook's contract is unchanged: an action that becomes unavailable after admission still leaves the original failed-step error authoritative.
- Cost: hosts now own a capability list that must track their dispatch arms. An over-claiming list rejects nothing at validation and still fails at dispatch (a `debug_assert` catches it in dev); an under-claiming one rejects a healthy job.
- Cost: the check is per-run rather than cached, and a workspace-local activity naming a genuinely new action must ship with a runtime that implements it.

## Provenance

Migrated verbatim from the local heading `activity-job/ADR-0252` in `docs/design/activity-job/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-07 · [ORB-10385]
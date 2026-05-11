## Context
The v2 job executor sub-modules (`step.rs`, `parallel.rs`, `fan_out.rs`, `loop_block.rs`, `target.rs`, `recovery.rs`) own non-trivial concurrency, ordering, and audit invariants. Without test coverage co-located with each block, regressions to those invariants surface only as production failures or as audit-trace anomalies that are hard to reproduce.

## Decision
Every executor-block module under `crates/orbit-engine/src/activity_job/job_executor/` gets a sibling `*_tests.rs` in `tests/` whose test function names name the specific invariant or failure mode each test guards. The current layout is `step_tests.rs`, `parallel_tests.rs`, `fanout_tests.rs`, `loop_tests.rs`, and `pipeline_durability_tests.rs`, alongside the pre-existing `audit_tests.rs`, `recovery_tests.rs`, and `target_tests.rs`. Shared scaffolding (`ScriptedHost`, `Action`, job/step builders) lives in `tests/mod.rs` so block modules stay focused on their own invariants and don't fork the host shape. Sandbox and policy boundary coverage lives next to the implementations they guard: `crates/orbit-exec/src/macos_sandbox.rs#tests` (read-deny enforcement and a realistic agent_loop profile boundary) and `crates/orbit-policy/src/engine.rs#tests` (global denyRead/denyModify last-match-wins, unknown-profile error, matched_rule observability).

## Consequences
- Future refactors of an executor block must keep the matching invariant test alive in the same-named test file or update it to reflect the new contract.
- New blocks (e.g. a future `dag` or `gate` construct) must land with a sibling test module covering at least the invariants enumerated in the seed surface.
- Shared scaffolding in `tests/mod.rs` is the consolidation seam — broaden it (agent_loop or shell hosts, additional builders) there rather than re-deriving in each block module.

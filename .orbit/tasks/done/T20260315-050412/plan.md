# Activity Asset Drift Investigation Plan

**Goal:** Decide and enforce the source-of-truth contract between bundled activity assets and tracked `.orbit/activities/active` copies.
**Scope:** Built-in activity assets, tracked active copies, init/seed/update flows, and tests that rely on either representation.
**Assumptions:** The current divergence is accidental unless explicitly documented otherwise.
**Risks:** Changing the wrong side without clarifying ownership could break init, runtime loading, or future maintenance workflows.

## Task 1: Audit divergence
1. Compare all tracked `.orbit/activities/active/*.yaml` files against their bundled `orbit-core/assets/activities/*.yaml` counterparts.
2. Classify differences as metadata-only vs behavioral/schema differences.

## Task 2: Choose the contract
1. Decide whether bundled assets or tracked active copies are canonical for built-in activities.
2. Document how the non-canonical side is derived or refreshed.

## Task 3: Enforce consistency
1. Update seeding/init/refresh logic and tests so the chosen contract is mechanically enforced.
2. Eliminate or intentionally preserve differences with explicit rationale.

## Final Verification
- `cargo test -p orbit-cli --test init_commands`
- `cargo test -p orbit-core`
- a repo diff/audit showing no unexplained asset/copy drift remains
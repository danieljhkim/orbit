Standardized the bundled asset YAML formatting under `orbit-core/assets` so activities, identities, and jobs now follow one readable layout with logical field grouping and consistent multi-line prose formatting.

Summary of changes:
- added section headers to bundled activity, identity, and job YAMLs so related fields are grouped consistently (`metadata`, `activity`, `identity`, `content`, `interface`, `execution`, `personality`, `behavior`, `job`)
- converted long prose fields to literal block scalars where appropriate, including all activity `instruction` fields and all bundled identity `description` fields
- normalized YAML list formatting in bundled assets for readability, such as `required`, `expected_exit_codes`, `skill_refs`, and command `args`
- preserved semantic content while reordering fields to a stable contract per asset family
- added a dedicated raw-text regression test in `orbit-core/tests/asset_formatting.rs` that locks in section ordering and block-scalar formatting for the current bundled asset set

Files touched:
- orbit-core/assets/activities/checkout_branch.yaml
- orbit-core/assets/activities/create_branch.yaml
- orbit-core/assets/activities/dispatch_task.yaml
- orbit-core/assets/activities/implement_change.yaml
- orbit-core/assets/activities/open_pr.yaml
- orbit-core/assets/activities/oversee_orbit_operations.yaml
- orbit-core/assets/activities/perform_maintenance.yaml
- orbit-core/assets/activities/review_pr.yaml
- orbit-core/assets/activities/review_tasks.yaml
- orbit-core/assets/activities/run_tests.yaml
- orbit-core/assets/identities/lamport.yaml
- orbit-core/assets/identities/linus.yaml
- orbit-core/assets/identities/prii.yaml
- orbit-core/assets/identities/steve.yaml
- orbit-core/assets/jobs/job_oversee_orbit_operations.yaml
- orbit-core/assets/jobs/job_perform_maintenance.yaml
- orbit-core/assets/jobs/job_review_tasks.yaml
- orbit-core/assets/jobs/job_task_pipeline.yaml
- orbit-core/tests/asset_formatting.rs

Validation:
- cargo test -p orbit-core
- rg -n "instruction: \||description: \||# ---- metadata ----|# ---- activity ----|# ---- identity ----|# ---- execution ----|# ---- job ----|# ---- personality ----|# ---- behavior ----" orbit-core/assets

Notes:
- I kept this task scoped to the bundled asset files that currently exist in `orbit-core/assets`; older deleted asset files already present in the dirty worktree were left alone
- some current bundled asset files in this worktree are untracked (`review_tasks.yaml`, `job_review_tasks.yaml`, `job_task_pipeline.yaml`); I formatted them because they are part of the live bundled asset set here, but I did not try to resolve that separate tracking-state issue in this task
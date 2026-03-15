Aligned `orbit init` with the live bundled asset set so explicit initialization now refreshes all default activities and jobs, not just a stale subset of create-only entries.

Summary of changes:
- expanded bundled default activity seeding in `orbit-core/src/command/activity.rs` from the stale 4-entry list to all 10 shipped activity assets, and added refresh-on-explicit-init behavior for built-in activities
- expanded bundled default job seeding in `orbit-core/src/command/job.rs` to include `job_task_pipeline` and added an in-place built-in job refresh path for explicit init runs
- added internal activity/job update plumbing in `orbit-store` so init can restore bundled definitions without deleting run history or touching non-default artifacts
- updated `orbit-core/src/command/init.rs` and `orbit-cli/src/command/init.rs` so init reports `default_activities_refreshed` and `default_jobs_refreshed`, which matches the new refresh semantics better than the old created-only wording
- strengthened init regression coverage in `orbit-cli/tests/init_commands.rs` to prove fresh init creates all 10 activities and 4 jobs, explicit init restores tampered built-in activity/job files, and implicit bootstrap remains non-destructive
- tightened bundled asset enumeration tests in `orbit-core` and updated the legacy rename integration test to use the actual supported kebab-case review-task/job legacy names

Files touched:
- orbit-core/src/command/activity.rs
- orbit-core/src/command/init.rs
- orbit-core/src/command/job.rs
- orbit-cli/src/command/activity.rs
- orbit-cli/src/command/init.rs
- orbit-cli/tests/init_commands.rs
- orbit-cli/tests/job_commands.rs
- orbit-store/src/backend/contracts.rs
- orbit-store/src/backend/file_backends.rs
- orbit-store/src/file/activity_store.rs
- orbit-store/src/file/job_store.rs
- orbit-store/src/lib.rs
- orbit-types/src/event.rs

Validation:
- cargo test -p orbit-core
- cargo test -p orbit-cli --test init_commands
- cargo test -p orbit-cli --test job_commands
- cargo test --workspace

Notes:
- explicit `orbit init` now restores bundled built-in activity/job definitions in place, including reactivating built-in activities and resetting built-in job state/steps to the asset definition
- implicit initialization still only fills in missing defaults, so non-init commands do not overwrite local built-in edits opportunistically
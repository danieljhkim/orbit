Fixed the orphaned-running job-run bug by making `execute_activity_with_retries()` failure-safe after a run has already been marked `running`. If a later setup/orchestration error now bubbles out before normal completion, Orbit checks whether the run is still active and finalizes it to `failed` with a synthetic failed step record instead of leaving it stuck in `running`.

The runtime change is in `orbit-core/src/command/job.rs`. The fix preserves the original error returned to the caller while ensuring the persisted run transitions out of `running` and records a useful failure message.

Added a regression test in `orbit-core/tests/job_runtime_behavior.rs` that reproduces the exact pre-step failure shape: a job default input causes schema validation to fail after the run is already marked `running`, and the test now proves the run is stored as `failed` with a failed step and no longer appears in running-job views.

Validation:
- `cargo fmt --package orbit-core`
- `cargo test -p orbit-core --test job_runtime_behavior`
- `cargo test --workspace`
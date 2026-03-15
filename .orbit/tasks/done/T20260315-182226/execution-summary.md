Fixed the job pipeline input merge so agent steps contribute only their inner `result` payload to subsequent step input, while CLI and API steps continue to merge their flat JSON outputs unchanged.

Implemented a small helper in `orbit-core/src/command/job.rs` that inspects the activity spec type and extracts mergeable output appropriately:
- `agent_invoke` steps now merge `response_json.result`
- `cli_command` and `api` steps still merge `response_json` directly

Added a regression test in `orbit-core/tests/job_runtime_behavior.rs` that reproduces the original failure mode with a two-step job:
- step 1 is an agent activity returning `{"task_id":"T123"}` inside the success envelope
- step 2 is a `cli_command` activity that requires `task_id` in its input schema and echoes it back
- before the fix this failed with missing required property `task_id`
- after the fix the job succeeds and the second step sees `T123` at the top level

Validation:
- `cargo test -p orbit-core agent_step_result_fields_flow_into_next_step_input -- --nocapture`
- `cargo test --workspace`

Operational note:
- I intentionally did not run `orbit job run job_task_pipeline --input base=main` in this repo because the built-in pipeline performs branch creation, implementation, test, PR, and checkout side effects, and this worktree already contains unrelated user changes.
Confirmed the drift between bundled activity assets and tracked `.orbit/activities/active` copies has been resolved for the checked-in active activity set.

Verification summary:
- compared every file under `.orbit/activities/active` against its counterpart in `orbit-core/assets/activities`
- confirmed the overlapping activity definitions are semantically identical once generated `created_at` / `updated_at` metadata is ignored
- confirmed the earlier spec-type drift is gone (`agent_invoke` is now used consistently in the tracked active copies)
- confirmed remaining differences are limited to generated timestamps and YAML formatting/order, not runtime behavior
- ran `cargo test -p orbit-cli --test init_commands`
- ran `cargo test -p orbit-core`

Notes:
- `.orbit/activities/active` intentionally contains only a subset of bundled activities, so missing active copies for other bundled assets are not treated as drift in this task
- this task was resolved directly in the workspace before the proposal had been lifecycle-advanced, so the task was backfilled through backlog/in-progress/review for auditability
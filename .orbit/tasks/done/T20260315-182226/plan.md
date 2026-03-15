## Goal
Merge only `envelope.result` (the inner payload) into `current_input`, not the full `AgentResponseEnvelope`.

## Scope
- Fix the merge logic in the job pipeline step loop
- CLI and API step types already produce flat JSON results — no change needed for those
- No changes to activity/job YAML files

## Assumptions
- `envelope.result` is always an `Object` when present (enforced by output schema validation upstream)
- CLI and API steps store a flat JSON result directly in `response_json`, so they must NOT be affected

## Risks
- Must distinguish agent steps from CLI/API steps when extracting the merge payload
- Existing tests may rely on current (broken) envelope merge behaviour

## Task 1: Fix merge logic

**Files:**
- Modify: `orbit-core/src/command/job.rs` (lines ~300-309)

**Steps:**
1. Add a failing integration test that runs a two-step job where step 1 is an agent_invoke that returns `{task_id: "T123"}` and step 2 is a cli_command that requires `task_id` in its input — confirm test fails.
2. In the step loop, replace the current merge of `outcome.response_json` with extraction of the nested `result` object from the envelope for agent steps. For non-agent steps keep merging the flat `response_json` directly.
3. Re-run failing test — confirm it passes.
4. Run full test suite: `cargo test --workspace`

**Done When:**
- After a successful agent step, top-level keys from `envelope.result` are present in `current_input` for the next step
- Envelope wrapper fields (`schemaVersion`, `status`, `durationMs`) are NOT injected into `current_input`
- Existing CLI/API step merge behaviour is unchanged
- All workspace tests pass

## Final Verification
```
cargo test --workspace
orbit job run job_task_pipeline --input base=main
```
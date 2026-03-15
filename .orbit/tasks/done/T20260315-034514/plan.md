# Remove Tool-Side Parameter Aliasing

**Goal:** Each orbit builtin tool has one canonical required `id` param; activity instructions map their own input names to `id` at the call site.
**Scope:** `orbit-tools` Rust changes + three activity YAML instruction updates. No changes to `orbit-types` or `orbit-core`.
**Assumptions:** All callers of `orbit.task.show`, `orbit.task.update`, and `orbit.activity.show` are agent_invoke activities whose instructions can be updated.
**Risks:** Any external caller that passes `task_id` instead of `id` will get a "missing `id`" error after this change — but all known callers are the three activity YAMLs updated in this task.

## Task 1: Simplify orbit_id_params and required_string calls

**Files:**
- Modify: `orbit-tools/src/builtin/orbit/mod.rs`
- Modify: `orbit-tools/src/builtin/orbit/task_show.rs`
- Modify: `orbit-tools/src/builtin/orbit/task_update.rs`
- Modify: `orbit-tools/src/builtin/orbit/activity_show.rs`

**Steps:**
1. In `orbit_id_params`, remove the `{kind}_id` alias `ToolParam`. Return only `[{name: "id", required: true, ...}]`.
2. In `task_show.rs`: change `required_string(input, &["id", "task_id"], "id")` → `required_string(input, &["id"], "id")`.
3. In `task_update.rs`: same.
4. In `activity_show.rs`: change `required_string(input, &["id", "activity_id"], "id")` → `required_string(input, &["id"], "id")`.
5. Update tests in `mod.rs` that use `task_id`/`activity_id` aliases — change to use `id` key.
6. Run: `cargo test -p orbit-tools`

**Done When:** Tests pass. `orbit_id_params` returns a single-element vec. No `task_id` or `activity_id` keys appear in tool parameter declarations or `required_string` calls.

## Task 2: Update activity YAML instructions

**Files:**
- Modify: `orbit-core/assets/activities/open_pr.yaml`
- Modify: `orbit-core/assets/activities/implement_change.yaml`
- Modify: `orbit-core/assets/activities/review_pr.yaml`

**Steps:**
1. In `open_pr.yaml` step 1: change `orbit.task.show with task_id: "{{input.task_id}}"` → `orbit.task.show with id: "{{input.task_id}}"`.
2. In `implement_change.yaml`:
   - Step 1: `orbit.task.show with task_id:` → `orbit.task.show with id:`
   - Step 2: `orbit.task.update with task_id:` → `orbit.task.update with id:`
   - Step 7: `orbit.task.update with task_id:` → `orbit.task.update with id:`
3. In `review_pr.yaml` step 1: `orbit.task.show with task_id:` → `orbit.task.show with id:`.

**Done When:** No `task_id:` or `activity_id:` appears as a tool call argument inside any instruction block. Activity input schemas still use `task_id` as their own input field name — that is correct and unchanged.

## Final Verification
```
cargo test -p orbit-tools
cargo build --workspace
# Confirm dry-run no longer reports false missing params:
orbit tool dry-run orbit.task.show '{"task_id": "T123"}'  # should error: missing id
orbit tool dry-run orbit.task.show '{"id": "T123"}'        # should show no missing params
```
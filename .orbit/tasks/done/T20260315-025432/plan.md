# orbit.task.* Builtin Tools Implementation Plan

**Goal:** Add `orbit.task.show`, `orbit.task.list`, `orbit.task.update`, and `orbit.activity.show` as registered builtin tools.
**Scope:** New `orbit-tools/src/builtin/orbit/` module. No changes to other crates except registration wiring.
**Assumptions:** The `orbit` binary is on PATH in the agent execution environment. All tools use `run_process` and parse `--json` output from the CLI.
**Risks:** Output schema of `orbit task show --json` must stay stable — tools depend on it. If the CLI output changes, tools silently break.

---

## Task 1: Scaffold the orbit module

**Files:**
- Create: `orbit-tools/src/builtin/orbit/mod.rs`
- Modify: `orbit-tools/src/builtin/mod.rs`

**Steps:**
1. Create `orbit-tools/src/builtin/orbit/mod.rs` with a `register(registry: &mut ToolRegistry)` function (empty for now).
2. Add `pub mod orbit;` to `orbit-tools/src/builtin/mod.rs`.
3. Call `orbit::register(registry);` in `register_builtins`.
4. Run: `cargo check -p orbit-tools`

**Done When:** `cargo check` passes with the new empty module wired in.

---

## Task 2: Implement orbit.task.show

**Files:**
- Create: `orbit-tools/src/builtin/orbit/task_show.rs`
- Modify: `orbit-tools/src/builtin/orbit/mod.rs`

**Schema:**
- Input: `id: string` (required) — the task ID
- Output: the full task JSON as returned by `orbit task show <id> --json`

**Steps:**
1. Write a failing test: `task_show_rejects_missing_id`.
2. Implement `OrbitTaskShowTool` — runs `orbit task show <id> --json` via `run_process`, parses stdout as JSON, returns it.
3. Handle non-zero exit as `OrbitError::Execution` with stderr content.
4. Register in `orbit/mod.rs`.
5. Run: `cargo test -p orbit-tools task_show`

**Done When:** Tool is registered, rejects missing `id`, and returns parsed task JSON on success.

---

## Task 3: Implement orbit.task.list

**Files:**
- Create: `orbit-tools/src/builtin/orbit/task_list.rs`

**Schema:**
- Input: `status: string` (optional) — filter by status (e.g. `backlog`, `in-progress`)
- Output: JSON array of tasks as returned by `orbit task list --json [--status <status>]`

**Steps:**
1. Implement `OrbitTaskListTool` — runs `orbit task list --json`, appends `--status <status>` if provided.
2. Parse stdout as JSON array.
3. Register and test.

**Done When:** Tool returns all tasks without filter, and filters correctly when `status` is provided.

---

## Task 4: Implement orbit.task.update

**Files:**
- Create: `orbit-tools/src/builtin/orbit/task_update.rs`

**Schema:**
- Input: `id: string` (required), plus any subset of: `status`, `execution_summary`, `comment` (all optional strings)
- Output: updated task JSON

**Steps:**
1. Implement `OrbitTaskUpdateTool` — builds `orbit task update <id>` args from whichever optional fields are present.
2. Run with `--json` flag if available; otherwise parse `orbit task show` after update.
3. Register and test: update with `status` only, update with `comment` only, reject missing `id`.

**Done When:** Tool updates the specified fields and returns the updated task JSON.

---

## Task 5: Implement orbit.activity.show

**Files:**
- Create: `orbit-tools/src/builtin/orbit/activity_show.rs`

**Schema:**
- Input: `id: string` (required)
- Output: activity JSON as returned by `orbit activity show <id> --json`

**Steps:**
1. Implement `OrbitActivityShowTool` — runs `orbit activity show <id> --json`.
2. Register and test.

**Done When:** Tool is registered and returns parsed activity JSON.

---

## Task 6: Registration tests

**Files:**
- Modify: `orbit-tools/src/builtin/orbit/mod.rs` (add `#[cfg(test)]` block)

**Steps:**
1. Add a test `orbit_tools_are_registered` asserting all four tool names are present in a fresh `ToolRegistry`.
2. Run: `cargo test -p orbit-tools orbit_tools_are_registered`

**Done When:** All four tools appear in the registry test.

---

## Final Verification

```bash
cargo build --workspace
cargo test -p orbit-tools
orbit tool list  # should show orbit.task.show, orbit.task.list, orbit.task.update, orbit.activity.show
```
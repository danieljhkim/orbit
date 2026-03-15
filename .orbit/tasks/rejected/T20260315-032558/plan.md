# Tool Schema Alias Support Plan

**Goal:** Let tool schemas express required aliases so dry-run validation, activity instructions, and tool execution agree.
**Scope:** Tool schema metadata, dry-run validation, and any builtin tool docs/tests that need alias sets.
**Assumptions:** Existing tools mostly use single canonical parameter names, so the change should be additive.
**Risks:** Changing schema shape may affect CLI output and any consumers that assume each parameter is independently required.

## Task 1: Extend tool schema metadata

**Files:**
- Modify: `orbit-types/src/tool.rs`
- Modify: `orbit-tools/src/lib.rs` and/or schema emitters that construct `ToolSchema`

**Steps:**
1. Add a way to represent alias groups or "one-of" required parameter sets.
2. Keep backward compatibility for existing single-name required parameters.
3. Add shape tests if needed.

**Done When:** Tool schemas can describe alias-aware required inputs without duplicating or weakening validation.

## Task 2: Update dry-run validation

**Files:**
- Modify: `orbit-core/src/runtime/pipeline.rs` and any other dry-run callers

**Steps:**
1. Teach dry-run validation to treat alias groups as satisfied when any allowed key is present.
2. Preserve current missing-parameter reporting for non-aliased params.
3. Add tests for alias success and true missing-input failure.

**Done When:** `run_tool_dry_run` reports missing params accurately for alias-aware tools.

## Task 3: Adopt the new schema in Orbit builtin tools

**Files:**
- Modify: `orbit-tools/src/builtin/orbit/mod.rs`
- Modify: `orbit-tools/src/builtin/orbit/*.rs`

**Steps:**
1. Update the new Orbit builtin tools to declare their canonical/alias ID inputs through the new schema mechanism.
2. Verify dry-run behavior matches actual execution behavior for `id` vs `task_id` / `activity_id`.

**Done When:** Alias-capable Orbit tools no longer have a schema/execution mismatch.

## Final Verification
- `cargo test -p orbit-tools`
- `cargo test -p orbit-core`
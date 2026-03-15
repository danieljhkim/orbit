# Bundled Asset YAML Formatting Plan

**Goal:** Make bundled activity, identity, and job YAML assets use one consistent, readable formatting style with predictable field grouping.
**Scope:** `orbit-core/assets/activities/*.yaml`, `orbit-core/assets/identities/*.yaml`, `orbit-core/assets/jobs/*.yaml`, plus any tests/helpers needed to lock in the formatting contract. Runtime behavior and non-asset YAMLs are out of scope unless a minimal test helper change is needed.
**Assumptions:** The semantic content of the assets should remain unchanged; this is primarily a readability and maintainability refactor. Long prose fields should use literal block style (`|`) when they are intentionally multi-line.
**Risks:** A purely manual cleanup can accidentally change serialized meaning, introduce inconsistent indentation, or let the style drift back without test coverage.

## Task 1: Define the canonical formatting contract

**Files:**
- Review: `orbit-core/assets/activities/*.yaml`
- Review: `orbit-core/assets/identities/*.yaml`
- Review: `orbit-core/assets/jobs/*.yaml`
- Inspect/modify as needed: tests near bundled asset parsing in `orbit-core/src/command/activity.rs`, `orbit-core/src/command/job.rs`, and identity loading

**Steps:**
1. Inventory the current formatting differences across bundled assets.
2. Define the canonical field order for each asset family so related fields are grouped together consistently.
3. Define the prose-formatting rule for long text fields, including when to prefer literal block style (`|`) over single-line or folded output.

**Done When:**
- there is a clear formatting contract for activities, identities, and jobs
- the contract is specific enough that another agent would not need to guess field order or block-scalar style

## Task 2: Reformat bundled activity, identity, and job assets

**Files:**
- Modify: `orbit-core/assets/activities/*.yaml`
- Modify: `orbit-core/assets/identities/*.yaml`
- Modify: `orbit-core/assets/jobs/*.yaml`

**Steps:**
1. Reorder fields in each asset to match the chosen logical grouping for its asset family.
2. Convert long prose fields to canonical literal block style where appropriate, especially `instruction` and long `description` content.
3. Keep values semantically unchanged while normalizing indentation, list style, and block formatting.
4. Verify that the resulting files still parse exactly as expected by Orbit.

**Done When:**
- bundled assets share one formatting style across all three asset families
- long prose fields are consistently represented in the agreed multi-line form
- no asset meaning changed as part of the formatting pass

## Task 3: Add regression coverage for formatting expectations

**Files:**
- Modify as needed: `orbit-core/src/command/activity.rs`
- Modify as needed: `orbit-core/src/command/job.rs`
- Modify/add tests near identity asset loading or asset snapshot/raw text checks

**Steps:**
1. Add or extend tests so bundled asset parsing still succeeds after the reformat.
2. Add at least one raw-text regression test or fixture assertion for the formatting contract where it adds value, especially for multi-line block style or required field order.
3. Keep the checks narrow enough that future intentional content edits are easy to update, while still preventing silent style drift.

**Done When:**
- tests would catch accidental regression to mixed formatting conventions
- the formatting contract is enforced by more than human memory

## Final Verification
- `cargo test -p orbit-core`
- `rg -n "instruction: \||description: \||schema_version:|schemaVersion:" orbit-core/assets`
- manual review of representative files in `orbit-core/assets/activities`, `orbit-core/assets/identities`, and `orbit-core/assets/jobs` to confirm consistent grouping and readability
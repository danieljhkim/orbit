# Remove orbit-maintain-system Plan

**Goal:** Eliminate the redundant `orbit-maintain-system` skill and leave Orbit with a smaller, clearer built-in skill surface.
**Scope:** Built-in skill assets, activity references, skill catalogs, and tests that currently expose or depend on `orbit-maintain-system`.
**Assumptions:** Existing maintenance workflows can be expressed through the remaining Orbit skills and direct activity instructions.
**Risks:** Built-in maintenance jobs may fail or become confusing if their instructions are not updated in lockstep with the skill removal.

## Task 1: Remove the skill from built-in assets and registries

**Files:**
- Modify: `orbit-core/src/command/skill.rs`
- Delete or modify: `orbit-core/assets/skills/orbit-maintain-system/SKILL.md`
- Delete or modify: `.orbit/skills/orbit-maintain-system/SKILL.md`
- Modify: `orbit-core/assets/skills/orbit-skills/SKILL.md`
- Modify: `.orbit/skills/orbit-skills/SKILL.md`

**Steps:**
1. Remove the embedded/seeded registration for `orbit-maintain-system`.
2. Delete the skill asset files or otherwise stop shipping them.
3. Update the skill catalog docs so the skill is no longer advertised.

**Done When:**
- Orbit no longer ships or lists `orbit-maintain-system` as an available built-in skill.
- Fresh skill initialization does not recreate it.

## Task 2: Rewire activities and docs that depend on the skill

**Files:**
- Modify: `orbit-core/assets/activities/perform-maintenance.yaml`
- Modify: `.orbit/activities/active/perform-maintenance.yaml`
- Modify: any related docs or assets that instruct use of `orbit-maintain-system`

**Steps:**
1. Replace the activity instruction/skill refs so `perform-maintenance` no longer depends on the removed skill.
2. Keep maintenance guidance concise and aligned with the remaining Orbit workflow.
3. Remove any dangling references from task templates, docs, or seeded assets.

**Done When:**
- No production activity or shipped doc references `orbit-maintain-system`.
- Built-in maintenance flows still have coherent instructions after the skill removal.

## Task 3: Update tests and seeded-workspace expectations

**Files:**
- Modify: `orbit-cli/tests/init_commands.rs`
- Modify: any core/CLI tests that assert the skill exists
- Modify: any fixtures or generated expectations tied to the removed skill

**Steps:**
1. Remove test expectations that seeded workspaces include `orbit-maintain-system`.
2. Add or update coverage proving the skill catalog/init output stays consistent after removal.
3. Run the affected test suites.

**Done When:**
- Tests pass without expecting the removed skill.
- There are no dangling seeded-workspace assertions for `orbit-maintain-system`.

## Final Verification
- `cargo test -p orbit-core`
- `cargo test -p orbit-cli`
- `cargo test -p orbit`
# Fix Stale Path in orbit-execute-change-request Skill

**Goal:** Ensure agents never write execution summaries to the wrong location.
**Scope:** `~/.claude/skills/orbit-execute-change-request/SKILL.md` and the canonical asset. No code changes.
**Assumptions:** The canonical asset at `orbit-core/assets/skills/` is authoritative.
**Risks:** None — documentation-only change.

## Task 1: Update user-level deployed skill

**Files:**
- Modify: `~/.claude/skills/orbit-execute-change-request/SKILL.md`

**Steps:**
1. Replace the hardcoded `/Users/daniel/.orbit/tasks/<status>/<id>/execution-summary.md` path with an instruction to use `orbit task update --execution-summary` instead of a direct file write.
2. Add a note: the CLI resolves the correct bundle path — agents must not hardcode or guess it.

**Done When:**
- No absolute `~/.orbit` path appears in the user-level skill.

## Task 2: Align canonical asset

**Files:**
- Modify: `orbit-core/assets/skills/orbit-execute-change-request/SKILL.md`

**Steps:**
1. Verify `{{ORBIT_ROOT}}` usage is still correct and unambiguous.
2. Add explicit instruction to prefer `orbit task update --execution-summary` over direct file writes.

**Done When:**
- Canonical asset instructs agents to use the CLI, not a file path.

## Final Verification
- grep -r 'daniel/.orbit' ~/.claude/skills/ — must return no matches
- Read both skill files and confirm no hardcoded absolute paths remain
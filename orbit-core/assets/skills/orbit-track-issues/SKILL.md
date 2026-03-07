---
name: orbit-track-issues
description: Use this skill when issues are identified by agents or humans. All issues must be tracked. Use this to track issues properly.
---

# Track Issues

Use this skill to evaluate and maintain issue lifecycle discipline while synchronizing each issue with an Orbit task.

Ensure:

- No pre-existing pending orbit issue already covers the same concern.
- The issue is clearly defined
- The implementation aligns with issue intent
- Status fields reflect reality
- Risks and assumptions are documented
- Next actions are explicit
- Lifecycle state is disciplined

This skill does not implement product changes; it performs governance and tracking updates.

---

## Orbit Task Contract

Create and manage Orbit tasks directly with `orbit task` commands.

Requirements:

- Every identified issue MUST have one Orbit task with `--type issue`.
- Task description MUST include description of the problem and the impact.
- Task instruction MUST include concrete steps to your recommended next actions
- Assign task priority based on the risk level assessment - `low`, `medium`, `high`

For canonical `orbit task add` instruction, refer to the `orbit-manage-tasks` skill.

---

## Completion Standard

Tracking is complete when:

- No duplicate Orbit issue task exists for the same concern.
- Orbit issue task is created aligned to the orbit task contract.

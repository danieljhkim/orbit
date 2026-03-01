---
name: orbit-maintain-system
description: Perform routine, low-risk maintenance to keep the system healthy, consistent, and up-to-date without changing intended behavior. Use this skill only when explicitly requested. 
---

# orbit-maintain-system

Use this skill when a 

---

## Inputs
- Repository workspace (cwd)
- Maintenance policy or schedule
- Optional scope constraints (paths, components)

---

## Responsibilities
1. Update dependencies within allowed ranges
2. Normalize formatting and lint issues
3. Remove dead code or unused assets
4. Apply safe migrations or minor upgrades
5. Verify system integrity after changes

---

## Non-Goals
- Implement new features (use `orbit-execute-change-request`)
- Diagnose unknown failures (use `investigate-issue`)
- Large architectural redesign

---

## Execution Contract
- Preserve observable behavior
- Prefer minimal, incremental changes
- Avoid breaking API or schema contracts
- Abort on validation failure
- Produce reviewable diffs

---

## Output
Return exactly one JSON object:

```json
{
  "status": "success|failed",
  "summary": "maintenance performed",
  "actions": [],
  "files_modified": [],
  "validation": {
    "build": "pass|fail|skipped",
    "tests": "pass|fail|skipped",
    "lint": "pass|fail|skipped"
  },
  "notes": "follow-ups or risks"
}
```

---

## Exit Criteria
- Maintenance actions completed or safely skipped
- Validation completed
- JSON result emitted

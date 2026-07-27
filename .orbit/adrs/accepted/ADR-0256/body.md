## Context
Top-level nav is the dashboard's scarcest surface, and two of its six entries were not earning a slot: a deprecated review-threads tab with no backing view, and Scoreboard, a diagnostics-shaped read-only telemetry view sitting beside the operator workflow tabs.

## Decision
The top-level nav is exactly Tasks, Audit, Diagnostics, Knowledge (plus the hash-only run-detail route). The deprecated tab is removed outright rather than hidden. Scoreboard becomes a Diagnostics subtab routed as #diagnostics/scoreboard, with its markup moved verbatim so the /api/scoreboard contract is untouched.

## Consequences
- The nav reads as the operator workflow; telemetry lives one level down.
- Existing #scoreboard bookmarks fall back to Tasks.
- Cost: the diagnostics pane owns two main elements and a visibility toggle keyed on the active subtab.
---
provenance: Verbatim copy of the task description + acceptance criteria given to all four planner agents.
shared_by: ORB-00154 and ORB-00155 (description.md / acceptance.md are byte-identical between the two task workspaces)
source: ~/.orbit/tasks/workspaces/orbit-8fb91e/ORB-00154/description.md and acceptance.md
recorded: 2026-05-18
---

# Task description

## Problem

The Scoreboard tab in the Orbit dashboard renders **six separate tables** stacked vertically: Delivery, Review, Knowledge, Operations, Planning Duels, and Attribution Cleanup (see `renderScoreboard` at [crates/orbit-dashboard/assets/dashboard/app.js:745](crates/orbit-dashboard/assets/dashboard/app.js:745)). Concrete UX failures visible in the current design:

1. **Cross-table comparison is effectively impossible at a glance.** The same four canonical agents (`codex`, `claude`, `gemini`, `grok`) appear as repeated rows in every section. To answer "which agent is leading overall?" the user has to scan 4×6 = 24 rows across six tables with **inconsistent column counts** (Delivery: 2 metrics, Review: 1 metric, Knowledge: 4 metrics, Operations: 3 metrics, Planning Duels: variable, Attribution Cleanup: union).
2. **Heavy zero-noise.** Sparse activity renders as dimmed `0` cells everywhere (`table.scoreboard-table td.zero { color: var(--border); }`). The dim tone reduces — but does not eliminate — the visual clutter; the user's eye still parses every cell.
3. **No relative-performance visualization.** Bare integers only. No bars, sparklines, color encoding, or rank badges. Comparing agents requires reading and comparing digit strings.
4. **Fixed row order, not performance order.** Rows are always in canonical insertion order (`CANONICAL_SCOREBOARD_FAMILIES = ["codex","claude","gemini","grok"]`), regardless of who is leading. No sorting controls.
5. **Repeated headers.** Each section repeats its own `AGENT` column header plus per-metric headers, eating vertical space without adding information.
6. **Unlabeled top-right counter.** The `scoreboard-count` element shows a bare number (e.g. `21`) with no label — a user cannot tell whether it is rows, refresh interval, total events, or something else.
7. **Stacked-table layout consumes far more vertical scroll than the data warrants.** Six full table cards for four agents could be a single grid.

## Why It Matters

The Scoreboard is the public-facing "who's winning" view of the multi-agent system and the primary signal users use to judge agent performance. The current design buries that signal in chrome.

## Instruction to Planner Agents (Planning Duel)

**This task is intended as a planning-duel seed.** Multiple planner agents (codex, claude, gemini, grok) should each propose their own complete redesign in the `plan` field of their duel-submitted plan. Do *not* converge on a single layout in advance — each planner should argue for their preferred shape and explain the tradeoffs. The winning plan is selected via the standard duel-selection process.

Each plan must:

- Name the chosen layout shape (one specific design — single unified table, card grid, sparkline matrix, heatmap, etc.) — not a menu of options.
- Specify how the **six current metric groups** (Delivery, Review, Knowledge, Operations, Planning Duels, Attribution Cleanup) collapse, consolidate, or re-group.
- State explicitly how **zero values** render (omit, em-dash, dot, faded, etc.).
- State how **relative performance** is visually encoded (color scale, bar length, sparkline, rank badge, percentile, etc.).
- State whether and how **sorting** works (clickable headers, default sort metric, primary-metric leader-board ordering, etc.).
- Identify any new client state required in `app.js` (e.g., active sort column) and where it lives.
- Include a **wire-level sketch** (ASCII art or markdown table) of the proposed layout so reviewers can compare designs side-by-side.
- Name at least two **rejected alternatives** with one-line rationale each — these feed the ADR.

## Constraints / Notes

- Canonical files to modify: [crates/orbit-dashboard/assets/dashboard/app.js](crates/orbit-dashboard/assets/dashboard/app.js) (`renderScoreboard` and helpers, ~lines 640–820) and [crates/orbit-dashboard/assets/dashboard/index.html](crates/orbit-dashboard/assets/dashboard/index.html) (scoreboard CSS).
- The orphan files at `crates/orbit-cli/assets/dashboard/` are dead code left over from ORB-00146 — do **not** modify them; they will be removed in a separate cleanup task.
- The redesign should **not** require a new backend endpoint. The existing `summary` payload that `renderScoreboard(summary)` consumes is sufficient. If the planner believes a new aggregate is genuinely needed, they must justify it.
- Stay within the dashboard's existing dark-theme tokens (`--accent`, `--accent-hover`, `--border`, etc.).
- Browser verification at **1440×900** is the canonical viewport.

## Out of Scope

- Backend changes to the scoreboard data shape.
- Cleanup of orphan files at `crates/orbit-cli/assets/dashboard/` (separate task).
- Splitting scoreboard rendering into its own ES module (`scoreboard.js`). That is a separate refactor task; this redesign lands inside `app.js` where the code currently lives.

# Acceptance criteria

- [ ] Opening the dashboard `/` at viewport 1440×900 and clicking the Scoreboard tab shows the redesigned layout with no horizontal scroll and no more than one screen of vertical content for the canonical 4-agent case (verified via `preview_resize` + `preview_screenshot`).
- [ ] Zero-value cells do not render as a visible `0` glyph in the canonical-agent rows (verified via `preview_snapshot`: text content of scoreboard rows for empty metrics contains no `0` character).
- [ ] A reader can identify the leading agent for any given metric by visual encoding alone (color, bar length, badge, sparkline, etc.) without comparing digit strings — verified by inspecting the rendered DOM via `preview_inspect` and confirming an inline visual element distinct from the numeric text exists for each metric cell.
- [ ] The previously bare `scoreboard-count` numeric badge either gains a visible accompanying label or is removed entirely (verified via `preview_snapshot`).
- [ ] The redesign keeps the canonical agent set (codex, claude, gemini, grok) as the primary row population, and any non-canonical "Attribution Cleanup" surface is rendered only when at least one non-canonical agent has non-zero signal (verified by inspecting the DOM with empty non-canonical state — no Attribution section present).
- [ ] An ADR is appended to [docs/design/user-interface/4_decisions.md](docs/design/user-interface/4_decisions.md) documenting the chosen layout, naming at least two rejected duel alternatives with one-line rationale each, and citing this task ID. The `**Last updated:**` line in the user-interface design docs is bumped in the same PR.
- [ ] `make ci-fast` passes on the implementing branch.

---
provenance: Verbatim copy of gemini's planner_b artifact for ORB-00155 (jrun-20260518-0620).
role: planner_b in ORB-00155; lost the duel to grok (planner_a)
arbiter: codex
gemini_model: gemini-3.1-pro-preview
source: ~/.orbit/tasks/workspaces/orbit-8fb91e/ORB-00155/artifacts/files/planning-duel/planner_b.md
recorded: 2026-05-18
---

*authored by: gemini / planner_b*
## 1. Hidden Coupling / Context
- `renderScoreboard` is consumed by `activeRefreshJobs` in `crates/orbit-dashboard/assets/dashboard/app.js`.
- The `renderScoreboardSection`, `renderScoreboardTable`, and `renderScoreboardCell` functions are mutually coupled and consumed solely by `renderScoreboard`.
- *Note:* The graph surfaces identical functions and usages inside `crates/orbit-cli/assets/dashboard/app.js` (e.g., `renderScoreboard` consumed by `activeRefreshJobs` there), but these are dead code per ORB-00155 constraints and should be ignored.

## 2. Plan
We will replace the six stacked tables with a single unified **Matrix Grid** layout.

### Symbols to Remove
- `renderScoreboardSection` (consumers: `renderScoreboard` in `crates/orbit-dashboard/assets/dashboard/app.js`)
- `renderScoreboardTable` (consumers: `renderScoreboardSection` in `crates/orbit-dashboard/assets/dashboard/app.js`)

### Symbols to Add
- `renderUnifiedScoreboardGrid`: A single function replacing the removed helpers, building one table with `<th colspan="...">` category headers.
- `currentScoreboardSort`: A new module-level state variable in `crates/orbit-dashboard/assets/dashboard/app.js` (e.g., `let currentScoreboardSort = { columnKey: null, dir: -1 };`).

### Symbols to Modify
- `renderScoreboard`: Modified to use the new `renderUnifiedScoreboardGrid`. It will continue to be called by `activeRefreshJobs`.
- `renderScoreboardCell`: Modified to return cells with the new visual encoding and dot for zero values.

### Implementation Details
- **Groups:** Delivery, Review, Knowledge, Operations, Planning, and Attribution Cleanup will be horizontally adjacent column groups under `colspan` headers. "Attribution Cleanup" columns only render if a non-canonical agent has data.
- **Zero values:** Instead of rendering `0`, cells with a zero value will render an empty dot `<span class="zero-dot">&middot;</span>` styled with `color: transparent` to completely remove visual clutter.
- **Relative performance:** A small, pure-CSS horizontal bar (`<div class="perf-bar" style="width: ${percent}%"></div>`) will be added to the cell background or adjacent flex-container, scaling to the maximum value in that column. The leading agent will receive a primary-color highlight.
- **Sorting:** Table headers will be clickable, updating `currentScoreboardSort` and calling `renderScoreboard(lastSummary)` to re-render.
- **Label:** The unlabeled top-right counter will be updated to display `count + " Agents"`.

## 3. Sketch

```text
+---------+------------------+-------------+
| AGENT   | DELIVERY         | KNOWLEDGE   |
|         | Tsk ↑ | PRs      | Art | Lrn   |
+---------+------------------+-------------+
| codex   | 14 ▄▄ | 5 ▄      | 3 ▄ |  ·    |
| claude  |  7 ▄  | 2        |  ·  | 1 ▄   |
| gemini  | 20 ▄▄▄| 8 ▄▄     | 5 ▄▄|  ·    |
| grok    |  ·    | ·        |  ·  |  ·    |
+---------+------------------+-------------+
```

## 4. Verification Commands
Run these exact commands to verify behavioral correctness:
- `rg 'renderScoreboardSection' crates/orbit-dashboard/assets/dashboard/app.js` (must return no results)
- `make ci-fast` (to ensure lint passes)
- `cargo run -p orbit-dashboard` and open `http://localhost:5173/` in a browser. Inspect the Scoreboard tab to verify sorting updates the table and zero values show no digit.

## 5. Context Files
- `crates/orbit-dashboard/assets/dashboard/app.js`
- `crates/orbit-dashboard/assets/dashboard/index.html`
- `docs/design/user-interface/4_decisions.md`

## 6. Risks & Rejected Alternatives
- **Risk:** Calculating maximums per column for the performance bars requires an extra pass over the data before rendering the rows.
- **Rejected Alternative 1 - Separate cards per metric:** Rejected because comparing all metrics for a single agent requires jumping across 6 cards, defeating the cross-metric comparison requirement.
- **Rejected Alternative 2 - Dropdown to select agent:** Rejected because the primary goal is cross-agent comparison; hiding agents in a dropdown prevents "at a glance" evaluation.
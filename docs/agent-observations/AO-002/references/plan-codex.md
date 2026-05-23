---
provenance: Verbatim copy of codex's planner_a artifact for ORB-00154 (jrun-20260518-0603).
role: planner_a in ORB-00154; lost the duel to claude (planner_b)
arbiter: gemini
source: ~/.orbit/tasks/workspaces/orbit-8fb91e/ORB-00154/artifacts/files/planning-duel/planner_a.md
recorded: 2026-05-18
---

*authored by: codex / planner_a*
## Plan

Chosen layout: **Agent Performance Matrix**. Replace the six stacked scoreboard sections with one canonical-agent matrix: one row for each `CANONICAL_SCOREBOARD_FAMILIES` entry and five grouped metric columns (`Delivery`, `Review`, `Knowledge`, `Operations`, `Planning Duels`). `Attribution Cleanup` becomes an optional compact secondary matrix rendered only when `otherRows.length > 0` after `scoreboardSignalForColumns(agent, OTHER_SCOREBOARD_COLUMNS) > 0`.

1. Hidden coupling found by `orbit.graph.refs` / `orbit.graph.search`:
   - Live caller of `renderScoreboard(summary)` is `activeRefreshJobs`, which calls `fetchJson("/api/scoreboard").then(renderScoreboard)` when `activeTab === "scoreboard"`.
   - `canonicalScoreboardRows(agentsMap)` is consumed by live `renderScoreboard`; `scoreboardSignalForColumns(agent, columns)` is consumed by live `renderScoreboard` for the non-canonical attribution gate.
   - `renderScoreboardSection(title, columns, rows)` is consumed by live `renderScoreboard`; `renderScoreboardTable(columns, rows, sectionTitle)` is consumed by `renderScoreboardSection`; `renderScoreboardCell(name, agent, col)` is consumed by `renderScoreboardTable`; `renderDuelMatrixSection(summary)` is consumed by live `renderScoreboard`.
   - `fmtScoreboardCount(value)` is consumed by live `formatScoreboardPair(agent, col)`, `renderScoreboardCell(name, agent, col)`, and `renderDuelMatrixSection(summary)`. `formatScoreboardPair(agent, col)` is consumed by live `scoreboardSignalForColumns(agent, columns)` and `renderScoreboardCell(name, agent, col)`. `asScoreboardNumber(value)` is consumed by live `fmtScoreboardCount`, `formatScoreboardPair`, `scoreboardSignalForColumns`, `renderScoreboardCell`, and `renderDuelMatrixSection`.
   - Graph refs also report duplicate symbols in `crates/orbit-cli/assets/dashboard/app.js`; the task marks those assets as orphaned dead code, so leave `crates/orbit-cli/assets/dashboard/` untouched. Run `git diff --stat -- crates/orbit-cli/assets/dashboard` before review and require no output.
   - `table.scoreboard-table` CSS is shared by `renderDiagnosticsTable`, `renderAudit`, and `renderRunEvents`; do not restyle that global selector for the new matrix. Add scoreboard-specific classes instead.
   - Existing proposed additions are unused today: `scoreboardSort`, `SCOREBOARD_METRIC_GROUPS`, `scoreboardMetricValue`, `scoreboardMetricMaxima`, `scoreboardOverallScore`, `scoreboardRowsForDisplay`, `setScoreboardSort`, `scoreboardHeaderCell`, `renderScoreboardMatrix`, `renderScoreboardMetric`, `renderAttributionCleanup`, and `headToHeadSummary` all returned no current graph-search matches.

2. In `crates/orbit-dashboard/assets/dashboard/app.js`, keep `CANONICAL_SCOREBOARD_FAMILIES`, `CANONICAL_SCOREBOARD_SET`, `DELIVERY_SCOREBOARD_COLUMNS`, `REVIEW_SCOREBOARD_COLUMNS`, `KNOWLEDGE_SCOREBOARD_COLUMNS`, `OPERATIONS_SCOREBOARD_COLUMNS`, `PLANNING_SCOREBOARD_COLUMNS`, `OTHER_SCOREBOARD_COLUMNS`, `readPath`, `asScoreboardNumber`, `fmtScoreboardCount`, `formatScoreboardPair`, `canonicalScoreboardRows`, and `scoreboardSignalForColumns`. Retire the stacked-section renderer identifiers `renderScoreboardSection`, `renderScoreboardTable`, `renderScoreboardCell`, and `renderDuelMatrixSection` after their logic is represented in the new matrix helpers.

3. Add `let scoreboardSort = { key: "overall", dir: "desc" };` near `runSort`. Add `const SCOREBOARD_METRIC_GROUPS = [...]` with explicit metric descriptors:
   - `delivery`: `tasks_created`, `tasks_planned`, `tasks_completed`
   - `review`: `task_review.threads`, `pr.review_comments`
   - `knowledge`: `knowledge.learnings_created`, `knowledge.learning_votes_received`, `knowledge.adrs_created`, `knowledge.adrs_accepted`, `knowledge.adrs_proposed_open`
   - `operations`: `graph_calls` from `agent?.tool_calls_by_surface?.graph ?? 0`, `task_calls` from `agent?.tool_calls_by_surface?.task ?? 0`, `failed_tool_calls`, `tool_calls`, `friction.reported`
   - `planning_duels`: `duels.wins`, `duels.losses`, `planner_runs`, `arbiter_runs`, `duel_decided`, and `head_to_head_record` derived from `summary.planning_duels.head_to_head.cells[family]`

4. Add these helpers in `app.js`: `scoreboardMetricValue(agent, metric, summary, family)`, `scoreboardMetricMaxima(rows, groups, summary)`, `scoreboardOverallScore(agent, groups, maxima, summary, family)`, `scoreboardRowsForDisplay(rows, groups, maxima, summary)`, `setScoreboardSort(key)`, `scoreboardHeaderCell(label, sortKey)`, `renderScoreboardMatrix(summary, rows, options)`, `renderScoreboardGroupCell(family, agent, group, maxima, summary)`, `renderScoreboardMetric(family, agent, metric, max, summary)`, `headToHeadSummary(summary, family)`, and `renderAttributionCleanup(summary, otherRows, maxima)`.

5. Rework `renderScoreboard(summary)` to set `$("scoreboard-count").textContent` to a labeled value such as `agents: ${entries.length}` or `canonical: ${CANONICAL_SCOREBOARD_FAMILIES.length}`. Preserve the empty state for `entries.length === 0`. Build `canonicalRows` exactly through `canonicalScoreboardRows(agentsMap)`. Build `otherRows` with the existing non-canonical filter and `scoreboardSignalForColumns(agent, OTHER_SCOREBOARD_COLUMNS) > 0`. Render `renderScoreboardMatrix(summary, canonicalRows, { scope: "canonical" })` plus `renderAttributionCleanup(summary, otherRows, maxima)` only when `otherRows.length > 0`.

6. Visual encoding: each metric item rendered by `renderScoreboardMetric` contains a non-text visual element `span.scoreboard-bar` with `style.width` set to `value / max * 100%`. The leading cell for a metric receives `data-leading="true"`, `.is-leading`, and `span.scoreboard-leader-badge` so the leader is identifiable by accent color and badge shape without reading the number. Numeric text stays in `span.scoreboard-value`.

7. Zero rendering: if `scoreboardMetricValue` returns zero for a numeric metric, `renderScoreboardMetric` renders `span.scoreboard-zero-dot` with text `·`, an empty/0-width bar, and no visible `0` glyph. For pair-like concepts, omit zero sides: `failed_tool_calls = 0` renders no failure number; `tool_calls > 0` can render `12 calls`; `duels.wins = 0` with `duels.losses > 0` renders the loss signal only; `0/0` renders `·`.

8. Sorting: default `scoreboardSort.key` is `overall`, computed by `scoreboardOverallScore` as the sum of normalized metric values across `SCOREBOARD_METRIC_GROUPS`. `scoreboardHeaderCell` renders compact buttons for `overall`, `tasks_completed`, `task_review.threads`, `knowledge.adrs_accepted`, `graph_calls`, and `duels.wins`; clicking calls `setScoreboardSort(key)` and toggles `dir`. `scoreboardRowsForDisplay` sorts canonical rows by the selected key and uses `CANONICAL_SCOREBOARD_FAMILIES` order as the stable tie breaker.

9. Wire sketch:

```text
Scoreboard                                      agents: 4
[overall ▼] [completed] [review] [accepted] [graph] [duels]
┌────────┬──────────────┬────────────┬──────────────┬──────────────┬──────────────┐
│ agent  │ Delivery     │ Review     │ Knowledge    │ Operations   │ Planning     │
├────────┼──────────────┼────────────┼──────────────┼──────────────┼──────────────┤
│ codex  │ completed █▉ │ threads ·  │ ADRs █ lead  │ graph ███    │ wins █ lead  │
│        │ planned █    │ PR rev ·   │ votes ▌      │ failures ·   │ vs peers ▌   │
├────────┼──────────────┼────────────┼──────────────┼──────────────┼──────────────┤
│ claude │ completed █  │ threads █  │ ADRs ▌       │ graph ▌      │ wins ▌       │
│ gemini │ ...          │ ...        │ ...          │ ...          │ ...          │
│ grok   │ ...          │ ...        │ ...          │ ...          │ ...          │
└────────┴──────────────┴────────────┴──────────────┴──────────────┴──────────────┘

Attribution Cleanup appears below this matrix only when non-canonical agents have non-zero signal.
```

10. In `crates/orbit-dashboard/assets/dashboard/index.html`, change the scoreboard panel header count from a bare `#scoreboard-count` number to a visible label-compatible badge. Add CSS for `.scoreboard-matrix`, `.scoreboard-matrix-row`, `.scoreboard-agent-cell`, `.scoreboard-group-cell`, `.scoreboard-metric`, `.scoreboard-bar-track`, `.scoreboard-bar`, `.scoreboard-value`, `.scoreboard-zero-dot`, `.scoreboard-leader-badge`, `.scoreboard-sortbar`, `.scoreboard-sort`, and `.scoreboard-attribution`. Keep `table.scoreboard-table` styles intact for Audit, Diagnostics, and Run Events.

11. Update `docs/design/user-interface/4_decisions.md` with a new ADR allocated by the Orbit ADR flow, citing `[ORB-00154]`. The ADR records the Agent Performance Matrix decision, the zero-dot rendering, visual bars/leader badges, default `overall` sorting, and the conditional Attribution Cleanup surface. Include rejected alternatives:
    - Rejected alternative: keep six stacked tables and add bars inside each table. Rationale: it preserves repeated headers and cross-table scanning.
    - Rejected alternative: four large agent cards with all metrics nested per card. Rationale: it makes metric-by-metric leader comparison harder than row-aligned scanning.
    - Rejected alternative: full heatmap-only grid. Rationale: color alone hides exact sparse counts and needs a heavier accessibility pass.
    Update the actual `last_updated` metadata used by the user-interface docs (`last_updated` in the inspected files), and update `docs/design/user-interface/2_design.md` to describe the matrix replacing grouped scoreboard sections.

12. Verification commands and checks:
    - `rg 'renderScoreboardSection|renderScoreboardTable|renderScoreboardCell|renderDuelMatrixSection' crates/orbit-dashboard/assets/dashboard/app.js` returns no matches after retiring the old section helpers.
    - `rg 'scoreboard-count|scoreboard-matrix|scoreboard-zero-dot|scoreboard-leader-badge|scoreboard-attribution' crates/orbit-dashboard/assets/dashboard/app.js crates/orbit-dashboard/assets/dashboard/index.html` shows the new labeled count, matrix hooks, zero marker, leader badge, and conditional attribution surface.
    - `git diff --stat -- crates/orbit-cli/assets/dashboard` returns no output.
    - `make ci-fast` passes.
    - Dashboard preview: open `/`, click the Scoreboard tab, run `preview_resize` with `1440×900`, then `preview_screenshot`; the canonical 4-agent matrix has no horizontal scroll and fits in one 900px-tall viewport.
    - Dashboard preview: run `preview_snapshot`; canonical scoreboard row text for empty metrics contains `·` or empty text and contains no visible `0` glyph for zero metrics.
    - Dashboard preview: run `preview_inspect` on `.scoreboard-metric`; each metric item contains `.scoreboard-bar` and leading metric items contain `.scoreboard-leader-badge` separate from `.scoreboard-value`.
    - Dashboard preview with no non-canonical signal: run `preview_inspect` for `.scoreboard-attribution`; the selector is absent.

## Context Files

- `crates/orbit-dashboard/assets/dashboard/app.js`: current scoreboard constants live around the `CANONICAL_SCOREBOARD_FAMILIES` block; `renderScoreboard` currently builds stacked `renderScoreboardSection(...)` calls plus `renderDuelMatrixSection(summary)`; `activeRefreshJobs` is the live `/api/scoreboard` caller.
- `crates/orbit-dashboard/assets/dashboard/index.html`: `#scoreboard-count` is currently a bare panel-header badge; `.scoreboard-table` is shared by non-scoreboard views, so new styles need `.scoreboard-matrix*` scope.
- `docs/design/user-interface/4_decisions.md`: existing ADR-0166 accepted the grouped sections now being superseded for scoreboard UX; append the new ADR after ADR-0167 and add `[ORB-00154]` to task references.
- `docs/design/user-interface/2_design.md`: current design prose still says the scoreboard groups metrics into separate sections; update it with the matrix layout in the same PR even though it was not in the seed context list.
- `crates/orbit-dashboard/src/api/scoreboard.rs` and `crates/orbit-store/src/file/scoreboard/scoreboard_summary.rs`: inspected only to confirm the existing summary payload already contains `agents`, dashboard extras, and `planning_duels.head_to_head`; no backend change belongs in this task.

## Risks

- The most likely regression is broad CSS drift because `.scoreboard-table` is reused by Audit, Diagnostics, and Run Events. Keep all new matrix styling under `.scoreboard-matrix*` selectors.
- Dense metric chips can become too tall if every metric label is rendered at full length. Use short labels from `SCOREBOARD_METRIC_GROUPS`, fixed row/grid dimensions, and wrapping inside group cells.
- The zero-dot rule needs pair metrics handled deliberately; formatting `0/N` or `0/0` reintroduces the exact glyph clutter the task removes.
- `overall` sorting can overweight high-volume metrics. Normalizing each metric against the canonical maximum keeps one large count from dominating every row.
---
provenance: Verbatim copy of grok's planner_a artifact for ORB-00155 (jrun-20260518-0620).
role: planner_a in ORB-00155; won the duel against gemini (planner_b)
arbiter: codex
source: ~/.orbit/tasks/workspaces/orbit-8fb91e/ORB-00155/artifacts/files/planning-duel/planner_a.md
recorded: 2026-05-18
---

*authored by: grok / planner_a*
# Planning Duel Proposal (grok / planner_a) — ORB-00155

## Plan

**1. Hidden coupling (must address first; discovered via graph before drafting)**
- `table.scoreboard-table` class used by renderDiagnosticsTable (app.js:1893), renderAudit (2776), renderRunEvents (3319) + 4 scoreboard renderers (via orbit.graph.search source_regex=scoreboard-table prefix=crates and orbit.graph.refs on renderScoreboardTable etc). Base table.scoreboard-table th,td rules (index.html:1075) must stay unchanged; new UI uses class scoreboard-leaderboard.
- The 6 *_SCOREBOARD_COLUMNS + OTHER (644–734) and renderScoreboardSection (795), renderScoreboardTable (807), renderScoreboardCell (836), renderDuelMatrixSection (868) have call sites only inside renderScoreboard (766–773) and each other (confirmed by orbit.graph.refs on the symbols and source_regex searches).
- Canonical consumers of renderScoreboard: activeRefreshJobs (app.js:2060: fetchJson /api/scoreboard .then(renderScoreboard)), setActiveTab path, refreshDashboard. No other files (graph search over crates/ with include_non_code found only the two app.js copies; ignore cli/ orphan per task).
- Reused unchanged: $ (33), el/syncNodes (common.js:8/77 imported at app.js:4), asScoreboardNumber (314), fmtScoreboardCount (318), formatScoreboardPair (323), readPath (736), compactCountFormatter (309), canonicalScoreboardRows (779), scoreboardSignalForColumns (783).

**2. Delete (post-rewrite dead code, 0 refs remain)**
- const DELIVERY_SCOREBOARD_COLUMNS (644)
- const REVIEW_SCOREBOARD_COLUMNS (651)
- const KNOWLEDGE_SCOREBOARD_COLUMNS (657)
- const OPERATIONS_SCOREBOARD_COLUMNS (666)
- const PLANNING_SCOREBOARD_COLUMNS (694)
- const OTHER_SCOREBOARD_COLUMNS (727)
- function renderScoreboardSection (795)
- function renderScoreboardTable (807)
- function renderScoreboardCell (836)
- function renderDuelMatrixSection (868)

**3. Add (new identifiers in app.js after readPath:743)**
- let lastScoreboardSummary = null;
- let scoreboardSortState = { key: "tasks_completed", dir: "desc" };
- const LEADERBOARD_COLUMNS = [{key:"agent",label:"agent",num:false},{key:"tasks_created",label:"created",num:true,title:"tasks created"},{key:"tasks_planned",label:"planned",num:true},{key:"tasks_completed",label:"completed",num:true},{key:"task_review.threads",label:"rev",num:true},{key:"pr.review_comments",label:"pr",num:true},{key:"knowledge.learnings_created",label:"learn",num:true},{key:"knowledge.learning_votes_received",label:"votes",num:true},{key:"knowledge.adrs_accepted",label:"adrs",num:true},{key:"graph_calls",label:"graph",num:true,compute:a=>a?.tool_calls_by_surface?.graph??0},{key:"tool_fail_all",label:"tool",num:true,format:"pair",left:"failed_tool_calls",right:"tool_calls"},{key:"duel_wl",label:"duel",num:true,format:"pair",left:"duels.wins",rightCompute:a=>(a?.duels?.wins??0)+(a?.duels?.losses??0),title:"wins / decided"}];
- function computeColumnMax(rows, col) { return Math.max(0, ...rows.map(r => asScoreboardNumber( col.compute ? col.compute(r[1]) : readPath(r[1], col.key) ))); }
- function isColumnLeader(rows, col, val) { return asScoreboardNumber(val) >= computeColumnMax(rows, col); }
- function renderPerfBar(val, max, isLeader) { const pct = max>0 ? Math.min(100, (asScoreboardNumber(val)/max)*100) : 0; return el("span", {class: `perf-bar${isLeader?" leader":""}`, style:{width:pct+"%", display:"inline-block", height:"3px", background: isLeader?"var(--accent)":"var(--border)", verticalAlign:"middle", marginRight:"3px", borderRadius:"1px"}}); }
- function setScoreboardSort(key) { if (scoreboardSortState.key === key) { scoreboardSortState.dir = scoreboardSortState.dir==="desc"?"asc":"desc"; } else { scoreboardSortState.key=key; scoreboardSortState.dir="desc"; } if (lastScoreboardSummary) renderScoreboard(lastScoreboardSummary); }
- function renderScoreboardLeaderboard(rows) { /* build table.scoreboard-leaderboard with sortable th onclick=setScoreboardSort, cells: agent (clickable) or (zero ? "–" : [renderPerfBar(v,max,isL), fmt...]) ; sort using scoreboardSortState */ ... return table; }
- function sortRows(rows, state) { /* stable numeric sort */ ... }

**4. Rewrite body of renderScoreboard (symbol:crates/orbit-dashboard/assets/dashboard/app.js#renderScoreboard:function lines 745-777)**
- Preserve signature, empty-state, count set.
- lastScoreboardSummary = summary;
- $("scoreboard-count").textContent = `${entries.length} agents`;
- const rows = [...canonicalScoreboardRows(agentsMap), ... filtered others];
- syncNodes($("scoreboard-body"), [el("div",{class:"scoreboard-table-wrap"}, [renderScoreboardLeaderboard(rows)])]);
- Delete the old sections array with 6 renderScoreboardSection + renderDuelMatrixSection calls.

**5. HTML change (index.html:2181)**
- header becomes <header><span>Scoreboard</span><span class="count-label" id="scoreboard-count">–</span></header> (JS sets "N agents" label)

**6. CSS additions (index.html after ~1061)**
- table.scoreboard-leaderboard { width:100%; border-collapse:collapse; font-size:11px; table-layout:fixed; }
- table.scoreboard-leaderboard th { cursor:pointer; }
- .perf-bar { display:inline-block; height:3px; background:var(--border); vertical-align:middle; margin-right:3px; border-radius:1px; }
- .perf-bar.leader { background:var(--accent); }
- td.leader-num { color:var(--accent-hi); font-weight:600; }
- Never mutate the existing table.scoreboard-table rules (used by renderDiagnosticsTable, renderAudit, renderRunEvents).

**7. ADR append (docs/design/user-interface/4_decisions.md)**
- Bump last_updated: 2026-05-18 to last_updated: 2026-05-19
- Insert before ## Task References the block:

## ADR-0168 — Unified Scoreboard Leaderboard with Perf Bars

**Status:** Accepted · 2026-05 · [ORB-00155]

**Context.** ADR-0166 (ORB-00144) split metrics into 6 stacked tables; cross-agent comparison and zero-noise violated the 1440×900 one-screen goal.

**Decision.** One sortable table (LEADERBOARD_COLUMNS) + inline perf bars (renderPerfBar) + en-dash zeros + per-col leader accent (isColumnLeader + .leader class) + module scoreboardSortState. Canonical rows always; non-canonical only on >0 signal (reuse scoreboardSignalForColumns). Duel matrix folded into duel_wl pair column.

**Consequences.**
- 4 rows + 12 cols fit <1 screen vertical, zero horiz scroll at 1440 (fixed layout + 11px).
- Leader per metric visible by bar length/color alone (no digit compare).
- scoreboard-count now reads "4 agents".
- Cost: separate Duel Matrix visual removed (W/L col remains).

Rejected alternatives: (1) per-agent card grid — rejected, same-metric scan still requires 4 separate cards. (2) colored heatmap cells only — rejected, loses exact counts required by fmtScoreboardCount and AC zero-glyph rule.

**8. Verification commands (exact; implementer executes these)**
- cargo check -p orbit-dashboard
- make ci-fast
- rg "^const (DELIVERY|REVIEW|KNOWLEDGE|OPERATIONS|PLANNING|OTHER)_SCOREBOARD_COLUMNS|^function render(ScoreboardSection|ScoreboardTable|ScoreboardCell|DuelMatrixSection)" crates/orbit-dashboard/assets/dashboard/app.js (must be 0 hits)
- rg "LEADERBOARD_COLUMNS|renderPerfBar|scoreboardSortState|perf-bar" crates/orbit-dashboard/assets/dashboard/app.js (≥5 hits)
- rg "scoreboard-leaderboard" crates/orbit-dashboard/assets/dashboard/index.html (≥1)
- 1440×900 browser AC checks via preview_resize 1440 900 + preview_screenshot + preview_snapshot (canonical rows text has 0 "0" chars) + preview_inspect (each metric cell has inline .perf-bar distinct from numeric text)
- git diff --name-only -- "crates/orbit-dashboard/assets/dashboard/app.js" "crates/orbit-dashboard/assets/dashboard/index.html" "docs/design/user-interface/4_decisions.md" (exactly those 3 files)

## Context Files

file:crates/orbit-dashboard/assets/dashboard/app.js
file:crates/orbit-dashboard/assets/dashboard/index.html
file:crates/orbit-dashboard/assets/dashboard/common.js
file:docs/design/user-interface/4_decisions.md

## Risks

- 12 fixed columns at 1440: implementer must run preview_resize and may drop 1-2 lowest cols (e.g. adrs_proposed) if bars cause any wrap.
- lastScoreboardSummary closure for sort-click re-render; fetch-then at activeRefreshJobs:2060 must populate it.
- Non-canonical rows gated on signal (AC).
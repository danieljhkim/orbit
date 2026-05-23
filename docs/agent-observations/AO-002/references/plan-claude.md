---
provenance: Verbatim copy of claude's planner_b artifact for ORB-00154 (jrun-20260518-0603).
role: planner_b in ORB-00154; won the duel against codex (planner_a)
arbiter: gemini
source: ~/.orbit/tasks/workspaces/orbit-8fb91e/ORB-00154/artifacts/files/planning-duel/planner_b.md
recorded: 2026-05-18
---

*authored by: claude / planner_b*
## Plan

### Step 1 — Hidden coupling enumerated up front

The scoreboard rendering is fully contained in `crates/orbit-dashboard/assets/dashboard/app.js`. Symbols and their call sites:

- `renderScoreboard` (line 745) — sole caller is `refreshDashboard` line 2032: `fetchJson("/api/scoreboard").then(renderScoreboard)`. Not exported.
- `canonicalScoreboardRows` (line 779) — only `renderScoreboard`.
- `scoreboardSignalForColumns` (line 783) — only `renderScoreboard`.
- `renderScoreboardSection` (line 795) — only `renderScoreboard`.
- `renderScoreboardTable` (line 807) — only `renderScoreboardSection`.
- `renderScoreboardCell` (line 836) — only `renderScoreboardTable`.
- `renderDuelMatrixSection` (line 868) — only `renderScoreboard`.
- Shared helpers `asScoreboardNumber` (line 314), `fmtScoreboardCount` (line 318), `formatScoreboardPair` (line 323), `readPath` (line 736) — also used by `renderDuelMatrixSection`. Keep all four.
- Column constants `DELIVERY_SCOREBOARD_COLUMNS` (644), `REVIEW_SCOREBOARD_COLUMNS` (651), `KNOWLEDGE_SCOREBOARD_COLUMNS` (657), `OPERATIONS_SCOREBOARD_COLUMNS` (666), `PLANNING_SCOREBOARD_COLUMNS` (694), `OTHER_SCOREBOARD_COLUMNS` (727) — keep verbatim; the redesign reuses every metric and `compute`/`format: "pair"` definition unchanged.
- `CANONICAL_SCOREBOARD_FAMILIES` (641), `CANONICAL_SCOREBOARD_SET` (642) — keep.
- `navigateToRole` (line 2692) — reused for the agent column-header click target.
- DOM helpers `el`, `syncNodes` from `./common.js` (line 4 import) — no edits.
- The class `scoreboard-table` is reused by NON-scoreboard tables: `app.js:1893` (locks panel `renderLocksPanel`), `app.js:2776` (audit table inside `renderAudit`), `app.js:3319` (diagnostics inside `renderDiagnosticsTable`). Therefore CSS rules under `table.scoreboard-table` (`index.html:1069–1110, 1574–1575`) MUST be preserved or migrated carefully — any rule deleted from there could regress audit/locks/diagnostics layouts.
- The bare `#scoreboard-count` element is only WRITTEN, never read by other JS: `rg 'scoreboard-count' crates/orbit-dashboard/assets/dashboard/` yields one writer in `renderScoreboard` line 750 and the placeholder `<span class="count" id="scoreboard-count">-</span>` at `index.html:2181`. Safe to relabel the textContent.
- `summary.planning_duels.head_to_head` payload shape (read by `renderDuelMatrixSection`) is unchanged.
- Orphan directory `crates/orbit-cli/assets/dashboard/` is OUT OF SCOPE per task description — DO NOT touch.

No cross-file imports of any scoreboard helper exist (`rg '^import.*(renderScoreboard|canonicalScoreboardRows|renderScoreboardSection|renderScoreboardTable|renderScoreboardCell|renderDuelMatrixSection)' crates/orbit-dashboard/assets/dashboard/` returns zero hits). All deletions are safe.

### Step 2 — Chosen layout: Unified Leaderboard Matrix (metric-major, single table)

ONE primary table replaces the six stacked tables. Axes:

- **Rows** = metrics. ~20 metric rows organized into 5 section-divider rows ("Delivery", "Review", "Knowledge", "Operations", "Planning Duels"). The section-divider row spans all columns with a small uppercase label, not a separate `<thead>`.
- **Columns** = `[metric-label, codex, claude, gemini, grok]` — fixed 5 columns. Canonical agents are columns; non-canonical agents NEVER appear in this table.
- **Sorting**: NO sortable headers and NO `activeSort` client state added. Each metric row is its own leaderboard; the per-row leader marker makes "who's winning this metric" instant. Cross-metric "who's leading overall" is read by counting `▲` markers visually along each agent column.
- **Visual encoding (AC3)**: every non-zero numeric metric cell renders an inline horizontal magnitude bar (`<span class="sb-bar">`) plus the numeric label. Bar width is `clamp(2px, value / row_max * 56px, 56px)` for `row_max >= 3`, and `min(value * 14px, 56px)` for `row_max < 3` so tiny rows do not visually exaggerate single-unit values. The row leader gets a `.sb-leader` class which appends a `▲` glyph via `::after` and shifts bar color to `var(--accent-hover)`. Ties: ALL tied-max cells get `▲` (truth-preserving).
- **Zero rendering (AC2)**: every cell with a numeric value of 0 — including the `left` half of a pair cell — renders as `<span class="sb-empty">—</span>` (em-dash U+2014). A pair cell where both halves are zero renders a single em-dash. A pair cell with left=0 and right>0 renders `<span class="sb-empty">—</span>/<span class="sb-pair-right">5</span>`. Result: no `0` digit ever appears in a canonical-agent row.
- **Pair cells** (`tool fail/all` from `OPERATIONS_SCOREBOARD_COLUMNS[3]`, `duel w/all` from `PLANNING_SCOREBOARD_COLUMNS[5]`): the `left` (failures / wins) value gets a tiny inline `▌`-style mini-bar scaled against the row's `row_max_left`, so every non-zero pair cell still has a distinct visual element separate from the numeric text (satisfies AC3).
- **Duel Matrix**: retained as a separate compact 4×4 table BELOW the leaderboard matrix, since head-to-head data does not fit the per-metric-row shape. `renderDuelMatrixSection` is kept unchanged.
- **Attribution Cleanup (AC5)**: rendered as a SECONDARY table below the duel matrix, only when `otherRows.length > 0`. Currently `renderScoreboard` always calls `renderScoreboardSection("Attribution Cleanup", ...)` which falls through to an `"No rows."` empty-state. Replace with `if (otherRows.length > 0) sections.push(buildLeaderboardMatrix(otherRows, ALL_SCOREBOARD_SECTIONS, { showSectionDividers: false }))`.

Wire-level sketch (1440×900 viewport, single screen target):

```
┌─ SCOREBOARD ─────────────────────────────────────────────── 4 agents ─┐
│ metric                  │  codex   │  claude  │  gemini  │  grok      │
├─────────────────────────┼──────────┼──────────┼──────────┼────────────┤
│ ── DELIVERY ──────────────────────────────────────────────────────── │
│ created                 │ ▌▌▌▌ 24▲ │ ▌▌ 11    │ ▌ 4      │   —        │
│ planned                 │ ▌▌▌ 18   │ ▌▌▌ 19▲  │ ▌▌ 9     │ ▌ 3        │
│ completed               │ ▌▌▌▌ 22▲ │ ▌▌ 12    │ ▌ 5      │   —        │
│ ── REVIEW ────────────────────────────────────────────────────────── │
│ review threads          │ ▌▌▌ 14   │ ▌▌▌▌ 19▲ │ ▌ 4      │   —        │
│ pr rev                  │   —      │ ▌▌ 6▲    │ ▌ 2      │   —        │
│ ── KNOWLEDGE ─────────────────────────────────────────────────────── │
│ learnings               │ ▌▌▌ 11   │ ▌▌▌▌ 17▲ │ ▌ 3      │ ▌ 2        │
│ votes                   │ ▌▌▌ 9    │ ▌▌▌▌ 13▲ │ ▌ 2      │   —        │
│ adrs                    │ ▌ 4      │ ▌▌ 6▲    │ ▌ 3      │   —        │
│ accepted                │ ▌ 3      │ ▌▌ 5▲    │ ▌ 2      │   —        │
│ proposed                │ ▌ 1▲     │   —      │   —      │   —        │
│ ── OPERATIONS ────────────────────────────────────────────────────── │
│ graph calls             │▌▌▌▌▌412▲ │ ▌▌ 180   │ ▌ 90     │   —        │
│ task calls              │ ▌▌ 88▲   │ ▌ 41     │ ▌ 27     │   —        │
│ tool fail/all           │  ▌3/412  │  ▌1/180  │   —/90▲  │   —        │
│ frict r                 │ ▌ 5      │ ▌▌ 8▲    │ ▌ 2      │   —        │
│ ── PLANNING DUELS ────────────────────────────────────────────────── │
│ wins                    │ ▌▌ 7     │ ▌▌▌ 9▲   │ ▌ 4      │ ▌ 3        │
│ losses                  │ ▌▌ 5     │ ▌▌▌ 6    │ ▌▌▌▌ 8   │ ▌▌▌▌▌ 9▲   │
│ as planner              │ ▌▌▌ 12   │ ▌▌▌ 15▲  │ ▌▌▌ 12   │ ▌▌▌ 12     │
│ as arbiter              │ ▌ 3▲     │ ▌ 2      │ ▌ 2      │ ▌ 1        │
│ duel w/all              │  ▌7/12   │  ▌9/14▲  │  ▌4/12   │  ▌3/10     │
└────────────────────────────────────────────────────────────────────────┘

┌─ DUEL HEAD-TO-HEAD (4×4) ─────────────────────────────────────────────┐
│  (existing renderDuelMatrixSection retained verbatim)                  │
└────────────────────────────────────────────────────────────────────────┘
```

Vertical budget at 1440×900: panel header ~50px + 5 section dividers × 20px + ~20 metric rows × 26px + table header 30px + duel matrix ~180px = ~880px. Fits under 900px with `make ci-fast` headroom.

### Step 3 — Symbols to ADD (in `crates/orbit-dashboard/assets/dashboard/app.js`)

- `const ALL_SCOREBOARD_SECTIONS` — ordered array of `{ title, columns }`, e.g. `[{ title: "Delivery", columns: DELIVERY_SCOREBOARD_COLUMNS }, ...]`. Inserted near line 734, immediately after `OTHER_SCOREBOARD_COLUMNS`.
- `function buildLeaderboardMatrix(rows, sectionList, opts)` — returns a `<table class="sb-leaderboard">` with one section-divider row per section title (skipped when `opts.showSectionDividers === false`), one metric row per non-`agent` column inside each section's `columns`, and a sticky header row with `[metric, ...rows.map(r => r[0])]`.
- `function rowMaxValue(rows, col)` — returns `Math.max(0, ...rows.map(([, agent]) => asScoreboardNumber(col.compute ? col.compute(agent) : readPath(agent, col.key))))`. For pair columns, returns max of `left` component only.
- `function metricCell(agent, col, rowMax, isLeader)` — emits `<td class="sb-metric-cell num">` containing either `<span class="sb-empty">—</span>` (when value === 0) or `<span class="sb-bar" style="width:Npx">` + `<span class="sb-value">N</span>` + optional `▲` via `.sb-leader` class on the `<td>`.
- `function pairMetricCell(agent, col, rowMaxLeft, isLeader)` — same shape, renders `<sb-bar>` scaled to `left/rowMaxLeft` next to `left/right` text; em-dash for either zero half.
- `function sectionDividerRow(title, columnCount)` — emits `<tr class="sb-section-divider"><td colspan="N">— TITLE —</td></tr>`.

### Step 4 — Symbols to MODIFY

- `renderScoreboard(summary)` (lines 745–777) — body becomes:
  ```js
  function renderScoreboard(summary) {
    const body = $("scoreboard-body");
    const agentsMap = (summary && summary.agents) || {};
    const entries = Object.entries(agentsMap);
    $("scoreboard-count").textContent = `${entries.length} agents`;
    if (entries.length === 0) { /* unchanged empty-state */ return; }
    const canonicalRows = canonicalScoreboardRows(agentsMap);
    const otherRows = entries
      .filter(([name]) => !CANONICAL_SCOREBOARD_SET.has(name))
      .filter(([, agent]) => scoreboardSignalForColumns(agent, OTHER_SCOREBOARD_COLUMNS) > 0)
      .sort(([a], [b]) => a.localeCompare(b));
    const sections = [
      buildLeaderboardMatrix(canonicalRows, ALL_SCOREBOARD_SECTIONS, { showSectionDividers: true }),
      renderDuelMatrixSection(summary),
    ];
    if (otherRows.length > 0) {
      sections.push(buildLeaderboardMatrix(
        otherRows,
        [{ title: "Attribution Cleanup", columns: OTHER_SCOREBOARD_COLUMNS }],
        { showSectionDividers: false },
      ));
    }
    syncNodes(body, [el("div", { class: "scoreboard-sections" }, sections)]);
  }
  ```

### Step 5 — Symbols to REMOVE

- `renderScoreboardSection` (lines 795–805)
- `renderScoreboardTable` (lines 807–834)
- `renderScoreboardCell` (lines 836–866)

Verify with: `rg -n 'renderScoreboardSection|renderScoreboardTable|renderScoreboardCell' crates/orbit-dashboard/` — expect zero hits after deletion.

### Step 6 — CSS changes in `crates/orbit-dashboard/assets/dashboard/index.html`

Lines 1026–1061 (`.scoreboard-sections`, `.scoreboard-section`, `.scoreboard-section-header`, `.scoreboard-section-title`, `.scoreboard-section-count`) — DELETE; they were only used by removed `renderScoreboardSection`.

Lines 1069–1110, 1574–1575 (`table.scoreboard-table` base rules, `td.zero`, `td.agent`, agent clickable hover) — KEEP. They are reused by `renderAudit`, `renderLocksPanel`, `renderDiagnosticsTable`.

Add new rules near line 1110:

```css
table.sb-leaderboard {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
  table-layout: fixed;
}
table.sb-leaderboard th, table.sb-leaderboard td {
  padding: 4px 12px;
  border-bottom: 1px solid var(--border);
  white-space: nowrap;
}
table.sb-leaderboard th {
  position: sticky; top: 0;
  background: var(--bg-elev);
  color: var(--fg-dim);
  font-size: 11px; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase;
}
table.sb-leaderboard td.sb-metric-cell {
  text-align: right;
  font-family: var(--font-mono);
}
.sb-bar {
  display: inline-block;
  height: 6px;
  background: var(--accent);
  border-radius: 2px;
  vertical-align: middle;
  margin-right: 6px;
}
.sb-leader .sb-bar { background: var(--accent-hover); }
.sb-leader::after { content: " ▲"; color: var(--accent-hover); font-size: 10px; margin-left: 2px; }
.sb-empty { color: var(--border); font-family: var(--font-mono); }
.sb-section-divider td {
  font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase;
  color: var(--fg-dim); background: #050505; padding: 6px 16px;
}
```

### Step 7 — ADR

Append `ADR-0168 — Unified Leaderboard Matrix Scoreboard` to `docs/design/user-interface/4_decisions.md` (after `ADR-0167`, before `## Task References`). Cite `[ORB-00154]`. Bump frontmatter `last_updated: 2026-05-17` in the same PR. Add `[ORB-00154]` line under `## Task References`.

Rejected alternatives (must include at least two; ADR will name three):

1. **Agent-major flat wide table** (4 rows, ~22 metric columns) — rejected: at 1440px viewport, 22 columns require horizontal scroll, violating AC1.
2. **Per-agent card grid (2×2)** — rejected: cards put each agent's metrics in a self-contained block, so "who's leading metric X?" still requires scanning across cards; preserves the original cross-section comparison failure mode.
3. **Pure heatmap matrix (color-coded cells, no numeric)** — rejected: precise values (e.g., "11 vs 12 learnings") matter for operator judgment; color-only encoding drops resolution.

### Step 8 — Verification commands (exact)

```bash
# 1. Confirm removed helpers are gone
rg -n 'renderScoreboardSection|renderScoreboardTable|renderScoreboardCell' crates/orbit-dashboard/
# Expect: zero hits

# 2. Confirm new symbols exist
rg -n 'buildLeaderboardMatrix|ALL_SCOREBOARD_SECTIONS|sb-leaderboard|sb-bar|sb-leader|sb-empty|sb-section-divider' crates/orbit-dashboard/assets/dashboard/
# Expect: definitions in app.js, CSS rules in index.html

# 3. Confirm no other JS reads scoreboard-count expecting bare digits
rg -n 'scoreboard-count' crates/orbit-dashboard/assets/dashboard/
# Expect: one writer in app.js (textContent assignment) + one placeholder in index.html — no readers

# 4. Lint / format gate
make ci-fast

# 5. Run dashboard and sanity-check the API payload still feeds the new renderer
cargo run -p orbit-cli -- web serve --no-open --port 7878 &
sleep 3
curl -s http://localhost:7878/api/scoreboard | jq '.agents | keys, (.agents.codex // {} | keys)'
# Expect: canonical families present; agent objects expose the same metric keys
#         (tasks_created, knowledge.*, tool_calls_by_surface, duels.*, etc.)

# 6. Browser verification at 1440×900 (cannot script from CLI; document explicitly)
#    - Open http://localhost:7878/, click Scoreboard tab.
#    - Confirm ONE primary leaderboard table replaces the six stacked tables.
#    - Confirm every non-zero metric cell shows an inline bar (.sb-bar).
#    - Confirm the row-leader cell shows ▲ via .sb-leader::after.
#    - Confirm zero cells display "—" (em-dash), never "0".
#    - Confirm the top-right count badge reads "4 agents" (or "N agents"), not bare "4".
#    - With no non-canonical agents present, confirm no Attribution Cleanup table is rendered.
#    - Confirm no horizontal scroll on .scoreboard-table-wrap at 1440px viewport width.
```

## Context Files

- `crates/orbit-dashboard/assets/dashboard/app.js` — column constants 641–734; `renderScoreboard` 745–777; `canonicalScoreboardRows` 779; `scoreboardSignalForColumns` 783; `renderScoreboardSection` 795 (DELETE); `renderScoreboardTable` 807 (DELETE); `renderScoreboardCell` 836 (DELETE); `renderDuelMatrixSection` 868 (KEEP); helpers `asScoreboardNumber` 314, `fmtScoreboardCount` 318, `formatScoreboardPair` 323, `readPath` 736 (KEEP); `navigateToRole` 2692 (KEEP); call site `refreshDashboard` line 2032 (no change).
- `crates/orbit-dashboard/assets/dashboard/index.html` — `<header>` for scoreboard panel at line 2181; CSS for scoreboard at lines 1021–1110 and 1574–1575; new CSS rules inserted near line 1110.
- `crates/orbit-dashboard/assets/dashboard/common.js` — `el`, `syncNodes` import source (no edits).
- `docs/design/user-interface/4_decisions.md` — append ADR-0168 after ADR-0167; bump `last_updated` frontmatter; extend `## Task References` with `[ORB-00154]` line.

## Risks

1. **Bar scaling distortion for low-magnitude rows.** A row `[1, 0, 0, 0]` would otherwise paint a full-width bar for value 1, exaggerating the lead. Mitigation is baked into `metricCell`: when `row_max < 3`, use a per-unit scale (`value * 14px` capped at 56px) instead of percentage. Document inline.
2. **Tie handling.** When two agents share row_max, both get `▲`. This is the chosen behavior — picking one alphabetically would lie about the data. Document in the ADR.
3. **AC2 strictness on pair cells.** `tool fail/all = 0/412` rendered as text contains a `0` glyph. Implementer MUST render the `left=0` half as `<span class="sb-empty">—</span>` to keep AC2 passing. The verification command for AC2 (preview_snapshot text content of empty cells) explicitly forbids any `0` glyph; assert this on the implementing branch with `rg -n 'sb-empty' crates/orbit-dashboard/assets/dashboard/app.js` and a manual snapshot.
4. **Vertical budget.** 20 metrics + 5 dividers + duel matrix is tight at 900px. Row padding `4px 12px` is the canonical density target. If the implementer measures >900px in the browser, drop the duel matrix into a collapsible block toggled by a small `[▾ matrix]` button, but DO NOT remove it — ADR-0166 (head-to-head matrix) is still accepted policy.
5. **CSS class reuse hazard.** `table.scoreboard-table` is reused by audit/locks/diagnostics. Renaming or deleting its base rules WILL regress those tables. The plan deliberately INTRODUCES a new `.sb-leaderboard` class instead of repurposing `.scoreboard-table`, leaving the shared CSS untouched.
6. **Removed `.scoreboard-section*` CSS still referenced by `renderScoreboard`'s wrapping `<div class="scoreboard-sections">`.** The plan keeps the wrapping `<div class="scoreboard-sections">` but the CSS rule for `.scoreboard-sections` is being deleted. Either keep the rule (as a no-op stack layout) or change the wrapper class to `.sb-stack { display:flex; flex-direction:column; gap:16px; }`. Decision: keep the wrapper class but reduce its CSS to a single `display: flex; flex-direction: column; gap: 16px;` rule (renamed `.sb-stack` is cleaner; implementer's choice — both satisfy AC1).
7. **ORB-00146 orphan dir trap.** Per task notes, `crates/orbit-cli/assets/dashboard/` is dead code from the dashboard-crate extraction. Implementer MUST NOT edit those files; verify with `git status -- crates/orbit-cli/assets/dashboard/` showing no changes before opening the PR.
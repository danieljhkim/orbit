// Orbit dashboard scoreboard-domain rendering.
// Pure vanilla JS, split into ES modules with no build step.

import { el, syncNodes, fetchJson } from './common.js';
import { navigateToRole } from './audit.js';

// ORB-00337: canonical scoreboard windows (mirror of
// `orbit_store::scoreboard_summary::ScoreboardWindow::as_str`). The
// boot fetch in app.js hardcodes `24h` to match the visually-highlighted
// segment; subsequent fetches happen from the selector click handler.
const SCOREBOARD_WINDOWS = ["1h", "24h", "7d", "30d", "all"];

function wireScoreboardWindowSelector() {
  const selector = document.getElementById("scoreboard-window-selector");
  if (!selector || selector.dataset.wired === "true") return;
  selector.dataset.wired = "true";
  selector.addEventListener("click", (event) => {
    const seg = event.target && event.target.closest(".scoreboard-window-seg");
    if (!seg || !selector.contains(seg)) return;
    const next = seg.dataset.window;
    if (!SCOREBOARD_WINDOWS.includes(next)) return;
    if (seg.classList.contains("on")) return; // no-op refetch
    for (const peer of selector.querySelectorAll(".scoreboard-window-seg")) {
      peer.classList.remove("on");
    }
    seg.classList.add("on");
    fetchJson(`/api/scoreboard?window=${encodeURIComponent(next)}`)
      .then(renderScoreboard)
      .catch((err) => {
        // Surface fetch failures in the console so the dashboard's "no
        // console errors" verification step catches regressions; the UI
        // keeps the previously-rendered scoreboard.
        console.error("scoreboard window refetch failed:", err);
      });
  });
}

const $ = (id) => document.getElementById(id);

const compactCountFormatter = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

function asScoreboardNumber(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function fmtScoreboardCount(value) {
  const num = asScoreboardNumber(value);
  return Math.abs(num) < 1000 ? String(num) : compactCountFormatter.format(num);
}

function formatScoreboardPair(agent, col) {
  const left = asScoreboardNumber(
    col.leftCompute ? col.leftCompute(agent) : readPath(agent, col.left),
  );
  const right = asScoreboardNumber(
    col.rightCompute ? col.rightCompute(agent) : readPath(agent, col.right),
  );
  return {
    left,
    right,
    text: `${fmtScoreboardCount(left)}/${fmtScoreboardCount(right)}`,
    zero: left === 0 && right === 0,
    title: `${col.title}: ${left} / ${right}`,
  };
}

const CANONICAL_SCOREBOARD_FAMILIES = ["codex", "claude", "gemini", "grok"];

const DELIVERY_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  { key: "tasks_created", label: "created", num: true },
  { key: "tasks_planned", label: "planned", num: true },
  { key: "tasks_completed", label: "completed", num: true },
];

const REVIEW_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  { key: "task_review.threads", label: "review threads", num: true },
  { key: "pr.review_comments", label: "pr rev", num: true },
];

const KNOWLEDGE_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  { key: "knowledge.learnings_created", label: "learnings", num: true },
  { key: "knowledge.learning_votes_received", label: "votes", num: true },
  { key: "knowledge.adrs_created", label: "adrs", num: true },
  { key: "knowledge.adrs_accepted", label: "accepted", num: true },
  { key: "knowledge.adrs_proposed_open", label: "proposed", num: true },
];

const OPERATIONS_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  {
    key: "graph_calls",
    label: "graph calls",
    num: true,
    compute: (agent) => agent?.tool_calls_by_surface?.graph ?? 0,
    title: "orbit.graph.* calls",
  },
  {
    key: "task_calls",
    label: "task calls",
    num: true,
    compute: (agent) => agent?.tool_calls_by_surface?.task ?? 0,
    title: "orbit.task.* calls",
  },
  {
    key: "tools",
    label: "tool fail/all",
    num: true,
    format: "pair",
    left: "failed_tool_calls",
    right: "tool_calls",
    title: "failed / total tool calls",
  },
  { key: "friction.reported", label: "frict r", num: true },
];

const PLANNING_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  { key: "duels.wins", label: "wins", num: true },
  { key: "duels.losses", label: "losses", num: true },
  {
    key: "planner_runs",
    label: "as planner",
    num: true,
    compute: (agent) => (agent?.duels?.wins ?? 0) + (agent?.duels?.losses ?? 0),
  },
  {
    key: "arbiter_runs",
    label: "as arbiter",
    num: true,
    compute: (agent) =>
      Math.max(
        0,
        (agent?.duels?.participated ?? 0) -
          ((agent?.duels?.wins ?? 0) + (agent?.duels?.losses ?? 0)),
      ),
  },
  {
    key: "duels",
    label: "duel w/all",
    num: true,
    format: "pair",
    left: "duels.wins",
    rightCompute: (agent) =>
      (agent?.duels?.wins ?? 0) + (agent?.duels?.losses ?? 0),
    title: "wins / decided duels (wins + losses)",
  },
];

const ALL_SCOREBOARD_SECTIONS = [
  { title: "Delivery", columns: DELIVERY_SCOREBOARD_COLUMNS },
  { title: "Review", columns: REVIEW_SCOREBOARD_COLUMNS },
  { title: "Knowledge", columns: KNOWLEDGE_SCOREBOARD_COLUMNS },
  { title: "Operations", columns: OPERATIONS_SCOREBOARD_COLUMNS },
  { title: "Planning Duels", columns: PLANNING_SCOREBOARD_COLUMNS },
];

function readPath(obj, path) {
  let cur = obj;
  for (const part of path.split(".")) {
    if (cur == null) return undefined;
    cur = cur[part];
  }
  return cur;
}

function renderScoreboard(summary) {
  // ORB-00337: idempotent attach of the window-selector click handler
  // (guarded internally so re-renders don't double-bind).
  wireScoreboardWindowSelector();

  const body = $("scoreboard-body");
  const narrativeHost = $("scoreboard-narrative");
  const duelHost = $("scoreboard-duel-matrix-host");
  const duelCount = $("scoreboard-duel-count");
  const insightsHost = $("scoreboard-insights");
  const insightsCount = $("scoreboard-insights-count");

  const agentsMap = (summary && summary.agents) || {};
  const entries = Object.entries(agentsMap);
  $("scoreboard-count").textContent = `${entries.length} agents`;

  if (entries.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No scoreboard data yet." })
    ])]);
    if (narrativeHost) syncNodes(narrativeHost, []);
    if (duelHost) syncNodes(duelHost, []);
    if (duelCount) duelCount.textContent = "—";
    if (insightsHost) syncNodes(insightsHost, []);
    if (insightsCount) insightsCount.textContent = "—";
    return;
  }

  const canonicalRows = canonicalScoreboardRows(agentsMap);
  const matrix = buildLeaderboardMatrix(canonicalRows, ALL_SCOREBOARD_SECTIONS, {
    showSectionDividers: true,
  });
  syncNodes(body, [el("div", { class: "scoreboard-sections" }, [matrix])]);

  // Narrative — single-line "claude leads creation · ..." summary. Skipped when
  // the cycle is too small to make a confident claim (< 10 created across the
  // canonical four).
  if (narrativeHost) {
    const narrative = renderScoreboardNarrative(summary);
    syncNodes(narrativeHost, narrative ? [narrative] : []);
  }

  // Duel matrix re-skin (CSS grid + per-cell w/l bar).
  if (duelHost) {
    const grid = renderDuelMatrixGrid(summary);
    syncNodes(duelHost, [grid]);
  }
  if (duelCount) {
    const families = summary?.planning_duels?.head_to_head?.families
      || CANONICAL_SCOREBOARD_FAMILIES;
    duelCount.textContent = `${families.length}×${families.length}`;
  }

  // Insights — rule-driven narrative cards. Panel collapses when no rule fires.
  if (insightsHost) {
    const panel = renderInsightsPanel(summary);
    syncNodes(insightsHost, panel ? [panel] : []);
    if (insightsCount) {
      const fired = panel ? panel.querySelectorAll(".ins-row").length : 0;
      insightsCount.textContent = fired === 0 ? "—" : String(fired);
    }
  }
}

function canonicalScoreboardRows(agentsMap) {
  return CANONICAL_SCOREBOARD_FAMILIES.map((family) => [family, agentsMap[family] || {}]);
}

function buildLeaderboardMatrix(rows, sectionList, opts = {}) {
  if (!rows.length) {
    return el("div", { class: "empty-state compact", text: "No rows." });
  }

  const showSectionDividers = opts.showSectionDividers !== false;
  const table = el("table", { class: "sb-leaderboard" });
  const thead = el("thead");
  const headRow = el("tr");
  headRow.appendChild(el("th", { class: "sb-metric-head", text: "metric" }));
  for (const [name] of rows) {
    const th = el("th", {
      class: "sb-agent-head clickable",
      text: name,
      title: `${name} — click to filter audit by role`,
    });
    th.addEventListener("click", () => navigateToRole(name));
    headRow.appendChild(th);
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = el("tbody");
  const columnCount = rows.length + 1;
  for (const section of sectionList) {
    if (showSectionDividers) {
      tbody.appendChild(sectionDividerRow(section.title, columnCount));
    }
    for (const col of section.columns.filter((candidate) => candidate.key !== "agent")) {
      const rowMax = rowMaxValue(rows, col);
      const tr = el("tr");
      tr.dataset.key = `scoreboard-${section.title}-${col.key}`;
      tr.appendChild(el("td", {
        class: "sb-metric-label",
        text: col.label,
        title: col.title || col.label,
      }));
      for (const [name, agent] of rows) {
        const value = scoreboardColumnValue(agent, col);
        const isLeader = rowMax > 0 && value === rowMax;
        const td = col.format === "pair"
          ? pairMetricCell(agent, col, rowMax, isLeader)
          : metricCell(agent, col, rowMax, isLeader);
        td.dataset.agent = name;
        td.dataset.metric = col.key;
        tr.appendChild(td);
      }
      tbody.appendChild(tr);
    }
  }
  table.appendChild(tbody);
  return table;
}

function rowMaxValue(rows, col) {
  return rows.reduce((max, [, agent]) => Math.max(max, scoreboardColumnValue(agent, col)), 0);
}

function scoreboardColumnValue(agent, col) {
  if (col.format === "pair") {
    return formatScoreboardPair(agent, col).left;
  }
  const value = col.compute ? col.compute(agent) : readPath(agent, col.key);
  return Math.max(0, asScoreboardNumber(value));
}

function metricCell(agent, col, rowMax, isLeader) {
  const value = asScoreboardNumber(col.compute ? col.compute(agent) : readPath(agent, col.key));
  const td = el("td", {
    class: `sb-metric-cell num${value === 0 ? " zero" : ""}${isLeader ? " sb-leader" : ""}`,
    title: `${col.title || col.label}: ${value}`,
  }, metricNodes(value, rowMax, isLeader));
  return td;
}

function pairMetricCell(agent, col, rowMax, isLeader) {
  const pair = formatScoreboardPair(agent, col);
  const td = el("td", {
    class: `sb-metric-cell num${pair.zero ? " zero" : ""}${isLeader ? " sb-leader" : ""}`,
    title: pair.title,
  }, [
    metricBar(pair.left, rowMax),
    el("span", { class: "sb-pair" }, pairTextNodes(pair.left, pair.right, pair.zero)),
    ...(isLeader ? [leaderBadge()] : []),
  ]);
  return td;
}

function metricNodes(value, rowMax, isLeader) {
  return [
    metricBar(value, rowMax),
    value === 0
      ? emptyScoreboardNode()
      : el("span", { class: "sb-value", text: fmtScoreboardCount(value) }),
    ...(isLeader ? [leaderBadge()] : []),
  ];
}

function metricBar(value, rowMax) {
  const num = Math.max(0, asScoreboardNumber(value));
  const width = num === 0 ? 6 : scaledMetricWidth(num, rowMax);
  return el("span", {
    class: `sb-bar${num === 0 ? " sb-bar-empty" : ""}`,
    style: { width: `${width}px` },
  });
}

function scaledMetricWidth(value, rowMax) {
  const max = Math.max(0, asScoreboardNumber(rowMax));
  if (max < 3) return Math.min(value * 14, 56);
  return Math.max(2, Math.round((value / max) * 56));
}

function leaderBadge() {
  return el("span", { class: "sb-leader-badge", text: "▲", title: "row leader" });
}

function emptyScoreboardNode() {
  return el("span", { class: "sb-empty", text: "—" });
}

function pairTextNodes(left, right, zero) {
  if (zero) return [emptyScoreboardNode()];
  return [
    left === 0 ? emptyScoreboardNode() : el("span", { class: "sb-value", text: fmtScoreboardCount(left) }),
    "/",
    right === 0 ? emptyScoreboardNode() : el("span", { class: "sb-pair-right", text: fmtScoreboardCount(right) }),
  ];
}

function sectionDividerRow(title, columnCount) {
  const tr = el("tr", { class: "sb-section-divider" });
  const td = el("td", { text: title });
  td.colSpan = columnCount;
  tr.appendChild(td);
  return tr;
}

// Re-skinned duel matrix: CSS grid, per-cell <w>–<l> score plus a horizontal
// two-segment bar whose widths are proportional to wins/losses. Diagonal cells
// are dimmed with an em-dash. Data source unchanged:
// `summary.planning_duels.head_to_head.cells[row][col]`.
function renderDuelMatrixGrid(summary) {
  const matrix = summary?.planning_duels?.head_to_head || {};
  const families = Array.isArray(matrix.families) && matrix.families.length
    ? matrix.families
    : CANONICAL_SCOREBOARD_FAMILIES;
  const cells = matrix.cells || {};

  const grid = el("div", { class: "scoreboard-duel-matrix" });
  grid.appendChild(el("span", { class: "dm-corner" }));
  for (const opponent of families) {
    grid.appendChild(el("span", { class: "dm-col-h", text: `vs ${opponent}` }));
  }

  for (const family of families) {
    const rowHeader = el("span", {
      class: "dm-row-h",
      text: family,
      title: `${family} — click to filter audit by role`,
    });
    rowHeader.addEventListener("click", () => navigateToRole(family));
    grid.appendChild(rowHeader);

    const row = cells[family] || {};
    for (const opponent of families) {
      if (family === opponent) {
        const diag = el("div", { class: "dm-cell diag" }, [
          el("span", { class: "dim", text: "—" }),
        ]);
        grid.appendChild(diag);
        continue;
      }
      const cell = row[opponent] || {};
      const wins = asScoreboardNumber(cell.wins);
      const losses = asScoreboardNumber(cell.losses);
      const runs = asScoreboardNumber(cell.runs);
      const total = wins + losses;
      const wPct = total > 0 ? (wins / total) * 100 : 0;
      const lPct = total > 0 ? (losses / total) * 100 : 0;

      const score = el("span", { class: "dm-score" }, [
        wins === 0
          ? el("span", { class: "dim", text: "0" })
          : el("span", { class: "w", text: String(wins) }),
        el("span", { class: "sep", text: "–" }),
        losses === 0
          ? el("span", { class: "dim", text: "0" })
          : el("span", { class: "l", text: String(losses) }),
      ]);
      const bar = el("div", { class: "dm-bar" }, [
        el("i", { class: "w", style: { width: `${wPct}%` } }),
        el("i", { class: "l", style: { width: `${lPct}%` } }),
      ]);
      const cellNode = el("div", {
        class: `dm-cell${runs === 0 ? " dm-empty" : ""}`,
        title: `${family} vs ${opponent}: ${wins} wins / ${losses} losses (${runs} runs)`,
      }, [score, bar]);
      grid.appendChild(cellNode);
    }
  }
  return grid;
}

// ===== Phase 1 additions: narrative + insights panel (rule-driven).
// Heuristics are intentionally simple so they can be audited at a glance and
// safely skip when the cycle is too small.

const NARRATIVE_MIN_CREATED_TOTAL = 10;

function canonicalAgentsList(summary) {
  const map = (summary && summary.agents) || {};
  return CANONICAL_SCOREBOARD_FAMILIES.map((family) => [family, map[family] || {}]);
}

function bestBy(rows, score) {
  let leader = null;
  let leaderScore = 0;
  for (const [name, agent] of rows) {
    const value = score(agent);
    if (value > leaderScore) {
      leaderScore = value;
      leader = name;
    }
  }
  return { name: leader, value: leaderScore };
}

/**
 * Render a one-line summary above the matrix. Returns the DOM node or `null`.
 *
 * Skip rule: total `tasks_created` across canonical families < 10. Quiet
 * cycles produce misleading narratives ("claude leads with 1 task"), so we
 * suppress the line entirely rather than render something dishonest.
 */
function renderScoreboardNarrative(summary) {
  const rows = canonicalAgentsList(summary);
  const totalCreated = rows.reduce(
    (sum, [, agent]) => sum + asScoreboardNumber(agent.tasks_created),
    0,
  );
  if (totalCreated < NARRATIVE_MIN_CREATED_TOTAL) return null;

  const segments = [];
  const pushSegment = (label, result) => {
    if (result && result.name && result.value > 0) {
      segments.push({ label, leader: result.name, value: result.value });
    }
  };
  pushSegment("creation", bestBy(rows, (a) => asScoreboardNumber(a.tasks_created)));
  pushSegment("planning", bestBy(rows, (a) => asScoreboardNumber(a.tasks_planned)));
  pushSegment("completion", bestBy(rows, (a) => asScoreboardNumber(a.tasks_completed)));
  pushSegment("duel wins", bestBy(rows, (a) => asScoreboardNumber(a?.duels?.wins)));

  if (segments.length === 0) return null;

  const children = [];
  segments.forEach((seg, idx) => {
    if (idx > 0) children.push(el("span", { class: "nar-sep", text: "·" }));
    children.push(el("b", { text: seg.leader }));
    children.push(document.createTextNode(` leads ${seg.label}`));
  });
  return el("div", { class: "scoreboard-narrative" }, children);
}

// ----- insight rules -----
// Each rule returns `{tone, headline, body}` or null. `body` is an array of
// DOM-nodes-or-strings so we can interleave agent pills.

function insightAgentPill(name) {
  return el("span", { class: `ag ${name}`, text: name });
}

// LEADER — top agent by `tasks_created`. Suppressed when nobody has created
// anything (handled implicitly via bestBy returning value 0).
function insightLeader(rows) {
  const top = bestBy(rows, (a) => asScoreboardNumber(a.tasks_created));
  if (!top.name || top.value === 0) return null;
  return {
    tone: "win",
    headline: "leader",
    body: [
      insightAgentPill(top.name),
      ` created `,
      el("b", { text: `${top.value} tasks` }),
      ` this window — the cycle's top author.`,
    ],
  };
}

// WATCH — agent whose friction.reported is >= 3× the team average (and has
// reported at least 3 to avoid noisy small-sample callouts).
function insightWatch(rows) {
  const counts = rows.map(([, a]) => asScoreboardNumber(a?.friction?.reported));
  const total = counts.reduce((s, v) => s + v, 0);
  if (total === 0) return null;
  const avg = total / rows.length;
  let pickName = null;
  let pickValue = 0;
  for (const [name, agent] of rows) {
    const value = asScoreboardNumber(agent?.friction?.reported);
    if (value >= 3 && value >= avg * 3 && value > pickValue) {
      pickValue = value;
      pickName = name;
    }
  }
  if (!pickName) return null;
  return {
    tone: "flag",
    headline: "watch",
    body: [
      insightAgentPill(pickName),
      ` reported `,
      el("b", { text: `${pickValue} friction events` }),
      ` — ~`,
      el("b", { text: `${(pickValue / Math.max(avg, 1)).toFixed(1)}×` }),
      ` the team average. Worth a quick look.`,
    ],
  };
}

// COLD — an agent with `duels.wins == 0 && duels.losses >= 3`. Calls out
// persistent zero-win streaks while ignoring small-sample 0-1 records.
function insightCold(rows) {
  for (const [name, agent] of rows) {
    const wins = asScoreboardNumber(agent?.duels?.wins);
    const losses = asScoreboardNumber(agent?.duels?.losses);
    if (wins === 0 && losses >= 3) {
      return {
        tone: "loss",
        headline: "cold",
        body: [
          insightAgentPill(name),
          ` is `,
          el("b", { text: `0–${losses}` }),
          ` in planning duels this window — never won as planner.`,
        ],
      };
    }
  }
  return null;
}

// SURPRISE — highest `tasks_completed / max(tasks_created, 1)` ratio among
// agents with `tasks_completed >= 5`. Surfaces "closers" — agents finishing
// far more than they author.
function insightSurprise(rows) {
  let pickName = null;
  let pickRatio = 0;
  let pickCompleted = 0;
  let pickCreated = 0;
  for (const [name, agent] of rows) {
    const completed = asScoreboardNumber(agent.tasks_completed);
    const created = asScoreboardNumber(agent.tasks_created);
    if (completed < 5) continue;
    const ratio = completed / Math.max(created, 1);
    if (ratio > pickRatio) {
      pickRatio = ratio;
      pickName = name;
      pickCompleted = completed;
      pickCreated = created;
    }
  }
  // Only call it a "surprise" if the closer finished meaningfully more than
  // they authored — otherwise it's just the same agent leading creation.
  if (!pickName || pickRatio < 1.5) return null;
  return {
    tone: "win",
    headline: "surprise",
    body: [
      insightAgentPill(pickName),
      ` completed `,
      el("b", { text: `${pickCompleted}` }),
      ` tasks against `,
      el("b", { text: `${pickCreated || 0}` }),
      ` created — ${pickRatio.toFixed(1)}× ratio, the cycle's closer.`,
    ],
  };
}

// COVERAGE — checks that all four canonical families have at least one
// activity signal (created, planned, completed, duel-participated, or any
// tool calls). Names the idle families when not.
function insightCoverage(rows) {
  const idle = rows
    .filter(([, agent]) => {
      const signal =
        asScoreboardNumber(agent.tasks_created) +
        asScoreboardNumber(agent.tasks_planned) +
        asScoreboardNumber(agent.tasks_completed) +
        asScoreboardNumber(agent?.duels?.participated) +
        asScoreboardNumber(agent.tool_calls);
      return signal === 0;
    })
    .map(([name]) => name);

  if (idle.length === 0) {
    return {
      tone: "win",
      headline: "role coverage",
      body: [`All four agent families are active this window. Healthy mix.`],
    };
  }
  const body = [];
  idle.forEach((name, idx) => {
    if (idx > 0) body.push(", ");
    body.push(insightAgentPill(name));
  });
  body.push(` ${idle.length === 1 ? "is" : "are"} idle this window — `);
  body.push(
    el("b", { text: `${idle.length} of ${rows.length}` }),
  );
  body.push(` canonical families silent.`);
  return {
    tone: idle.length >= 2 ? "flag" : "win",
    headline: "role coverage",
    body,
  };
}

/**
 * Compose the insights panel from all rules. Returns `<section
 * class="scoreboard-insights">` with one `.ins-row` per fired rule, or `null`
 * when nothing fires.
 */
function renderInsightsPanel(summary) {
  const rows = canonicalAgentsList(summary);
  const rules = [insightLeader, insightWatch, insightCold, insightSurprise, insightCoverage];
  const cards = rules
    .map((rule) => rule(rows))
    .filter((card) => card !== null);
  if (cards.length === 0) return null;

  const section = el("section", { class: "scoreboard-insights" });
  for (const card of cards) {
    const row = el("div", { class: `ins-row ${card.tone}` });
    row.appendChild(el("div", { class: "hd", text: card.headline }));
    const body = el("div", { class: "body" });
    for (const part of card.body) {
      if (typeof part === "string") {
        body.appendChild(document.createTextNode(part));
      } else if (part instanceof Node) {
        body.appendChild(part);
      }
    }
    row.appendChild(body);
    section.appendChild(row);
  }
  return section;
}

export { renderScoreboard, renderScoreboardNarrative, renderInsightsPanel, renderDuelMatrixGrid };

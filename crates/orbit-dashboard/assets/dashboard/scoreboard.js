// Orbit dashboard scoreboard-domain rendering.
// Pure vanilla JS, split into ES modules with no build step.

import { el, syncNodes } from './common.js';
import { navigateToRole } from './audit.js';

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
const CANONICAL_SCOREBOARD_SET = new Set(CANONICAL_SCOREBOARD_FAMILIES);

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

const OTHER_SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  ...DELIVERY_SCOREBOARD_COLUMNS.slice(1),
  ...REVIEW_SCOREBOARD_COLUMNS.slice(1),
  ...KNOWLEDGE_SCOREBOARD_COLUMNS.slice(1),
  ...OPERATIONS_SCOREBOARD_COLUMNS.slice(1),
  ...PLANNING_SCOREBOARD_COLUMNS.slice(1),
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
  const body = $("scoreboard-body");

  const agentsMap = (summary && summary.agents) || {};
  const entries = Object.entries(agentsMap);
  $("scoreboard-count").textContent = `${entries.length} agents`;

  if (entries.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No scoreboard data yet." })
    ])]);
    return;
  }

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
    sections.push(
      buildLeaderboardMatrix(
        otherRows,
        [{ title: "Attribution Cleanup", columns: OTHER_SCOREBOARD_COLUMNS }],
        { showSectionDividers: true },
      ),
    );
  }

  syncNodes(body, [el("div", { class: "scoreboard-sections" }, sections)]);
}

function canonicalScoreboardRows(agentsMap) {
  return CANONICAL_SCOREBOARD_FAMILIES.map((family) => [family, agentsMap[family] || {}]);
}

function scoreboardSignalForColumns(agent, columns) {
  return columns.reduce((total, col) => {
    if (col.key === "agent") return total;
    if (col.format === "pair") {
      const pair = formatScoreboardPair(agent, col);
      return total + (pair.zero ? 0 : 1);
    }
    const value = col.compute ? col.compute(agent) : readPath(agent, col.key);
    return total + Math.abs(asScoreboardNumber(value));
  }, 0);
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

function renderDuelMatrixSection(summary) {
  const matrix = summary?.planning_duels?.head_to_head || {};
  const families = Array.isArray(matrix.families) && matrix.families.length
    ? matrix.families
    : CANONICAL_SCOREBOARD_FAMILIES;
  const cells = matrix.cells || {};
  const rows = families.map((family) => [family, cells[family] || {}]);

  const section = el("section", { class: "scoreboard-section scoreboard-matrix-section" });
  section.appendChild(
    el("div", { class: "scoreboard-section-header" }, [
      el("span", { class: "scoreboard-section-title", text: "Duel Matrix" }),
      el("span", { class: "scoreboard-section-count", text: `${families.length}x${families.length}` }),
    ]),
  );

  const table = el("table", { class: "scoreboard-table duel-matrix-table" });
  const thead = el("thead");
  const headRow = el("tr");
  headRow.appendChild(el("th", { text: "family" }));
  for (const family of families) {
    headRow.appendChild(el("th", { class: "num", text: family }));
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = el("tbody");
  for (const [family, row] of rows) {
    const tr = el("tr");
    const label = el("td", {
      class: "agent clickable",
      text: family,
      title: `${family} — click to filter audit by role`,
    });
    label.addEventListener("click", () => navigateToRole(family));
    tr.appendChild(label);
    for (const opponent of families) {
      const cell = row[opponent] || {};
      const wins = asScoreboardNumber(cell.wins);
      const losses = asScoreboardNumber(cell.losses);
      const runs = asScoreboardNumber(cell.runs);
      tr.appendChild(el("td", {
        class: `num${runs === 0 ? " zero" : ""}`,
        title: `${family} vs ${opponent}: ${wins} wins / ${losses} losses (${runs} runs)`,
      }, pairTextNodes(wins, losses, runs === 0)));
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  section.appendChild(table);
  return section;
}

export { renderScoreboard };

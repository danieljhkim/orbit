// Orbit dashboard — terminal-dark, manually refreshed SPA.
// Pure vanilla JS, split into ES modules with no build step.

import { el, statusPill, stateCell, fetchJson, requestJson, postJson, patchJson, syncNodes, positiveIntParam } from './common.js';
import { buildChips, cacheCrewPayload, copyTaskIdWithNotice, hasCrewOptions, openVisibleTask, renderTasks, wireSearch } from './tasks.js';
import {
  applyAuditHashQuery,
  buildAuditChips,
  buildAuditHash,
  fetchAndRenderAudit,
  fetchAndRenderPolicy,
  getActiveAuditSubtab,
  navigateToAuditExecution,
  navigateToRole,
  renderAuditSummary,
  setActiveAuditSubtabFromButton,
  setAuditSubtab,
  syncAuditControls,
  wireAuditSearch,
} from './audit.js';
import { initLogTail, fitLogPanelToViewport } from './log-tail.js';
import {
  renderDiagnostics,
  renderImplementOneCard as renderImplOne,
} from './diagnostics.js';

const STATUS_ORDER = [
  "in-progress",
  "review",
  "blocked",
  "proposed",
  "backlog",
  "someday",
];

const DEFAULT_INACTIVE_STATUSES = new Set(["someday"]);
const STATUS_UPDATE_TARGETS = STATUS_ORDER
  .filter((status) => !["friction", "rejected", "archived"].includes(status))
  .concat(["done"]);

const JOB_RUN_LIMIT = positiveIntParam("runs", 25);
const DIAG_LIMIT = positiveIntParam("diag", 50);
const RUN_EVENTS_LIMIT = positiveIntParam("events", 100);
const LEARNING_LIMIT = positiveIntParam("learnings", 100);
const ADR_LIMIT = positiveIntParam("adrs", 100);
const FRICTION_LIMIT = positiveIntParam("frictions", 100);

const CANCELLABLE_RUN_STATES = new Set(["pending", "running"]);
const FRICTION_STATUSES = ["open", "triaged", "resolved"];

const $ = (id) => document.getElementById(id);

let searchQuery = "";
let activeStatuses = new Set(
  STATUS_ORDER.filter((s) => !DEFAULT_INACTIVE_STATUSES.has(s)),
);
let lastTasks = [];
let lastRuns = [];
let lastDiagnostics = { metrics: [], errors: [], implement_one: [] };
let lastLearningPayload = { stats: {}, items: [] };
let lastAdrPayload = { stats: {}, items: [] };
let lastFrictionPayload = { stats: {}, tags: [], items: [] };
let activeTab = "tasks";
let activeDiagSubtab = "runs";
let activeKnowledgeSubtab = "learnings";
let runSort = { key: "when", dir: "desc" };
let isRefreshing = false;
let activeLearningId = null;
let learningSearchQuery = "";
let activeAdrId = null;
let adrSearchQuery = "";
let activeFrictionId = null;
let frictionSearchQuery = "";

// Health strip state
let lastSummary = null;

// Run detail state
let activeRunId = null;
let activeRunDetail = null;
let activeRunEvents = [];
let activeRunLogs = [];
let activeRunSubtab = "steps";
let expandedStepIndices = new Set();

function taskContext() {
  return {
    getTasks: () => lastTasks,
    replaceTask: (updatedTask) => {
      const index = lastTasks.findIndex((task) => task.id === updatedTask.id);
      if (index >= 0) {
        lastTasks[index] = updatedTask;
      }
    },
    getSearchQuery: () => searchQuery,
    setSearchQuery: (value) => { searchQuery = value; },
    getActiveStatuses: () => activeStatuses,
    setActiveStatuses: (statuses) => { activeStatuses = statuses; },
    statusOrder: STATUS_ORDER,
    statusUpdateTargets: STATUS_UPDATE_TARGETS,
    fmtAbsTime,
    refreshDashboard,
  };
}

function auditContext() {
  return {
    navigateToRun,
    setActiveTab,
    refreshDashboard,
    fmtDuration,
    fmtTimestamp,
    fmtRelative,
    fmtAbsTime,
    truncate,
  };
}

function diagnosticsContext() {
  return {
    getLastDiagnostics: () => lastDiagnostics,
    getActiveDiagSubtab: () => activeDiagSubtab,
    fmtRelative,
    fmtDuration,
    truncate,
    setActiveTab,
    navigateToRun,
  };
}

function runIsCancellable(run) {
  return CANCELLABLE_RUN_STATES.has(run && run.state);
}

function buildCancelRunButton(run, host) {
  const btn = el("button", {
    class: "action reject run-cancel",
    text: "cancel",
    title: `Cancel ${run.run_id}`,
  });
  btn.disabled = !runIsCancellable(run);
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    cancelRun(run.run_id, btn, host);
  });
  return btn;
}

function buildReplayRunButton(run, host) {
  const btn = el("button", {
    class: "action approve run-replay",
    text: "Replay run",
    title: `Replay ${run.run_id}`,
  });
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    replayRun(run, btn, host);
  });
  return btn;
}

async function cancelRun(runId, btn, host) {
  if (!runId) return;
  const old = btn.textContent;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>cancel`;
  if (host) {
    for (const node of host.querySelectorAll(".action-error")) node.remove();
  }
  try {
    await postJson(`/api/runs/${encodeURIComponent(runId)}/cancel`);
    await Promise.all([
      fetchAndRenderRuns(),
      activeRunId === runId ? fetchAndRenderRunDetail() : Promise.resolve(),
      activeRunId === runId ? fetchAndRenderRunEvents() : Promise.resolve(),
    ]);
  } catch (e) {
    if (host) {
      host.appendChild(el("div", { class: "action-error", text: e.message || "cancel failed" }));
    }
    console.error(e);
  } finally {
    btn.disabled = false;
    btn.textContent = old;
  }
}

async function replayRun(run, btn, host) {
  const runId = run && run.run_id;
  if (!runId) return;
  if (run.state === "running" && !window.confirm(`Replay still-running run ${runId}?`)) return;
  const old = btn.textContent;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>Replay run`;
  if (host) {
    for (const node of host.querySelectorAll(".action-error")) node.remove();
  }
  try {
    const payload = await postJson(`/api/runs/${encodeURIComponent(runId)}/replay`);
    if (!payload.run_id) throw new Error("replay response did not include run_id");
    navigateToRun(payload.run_id);
    fetchAndRenderRuns().catch(console.error);
  } catch (e) {
    if (host) {
      host.appendChild(el("div", { class: "action-error", text: e.message || "replay failed" }));
    }
    console.error(e);
  } finally {
    btn.disabled = false;
    btn.textContent = old;
  }
}

function renderBodyBlock(body, fallbackClass) {
  if (!body || !body.trim()) return null;
  const isMarked = typeof marked !== "undefined";
  const view = el(isMarked ? "div" : "pre", {
    class: isMarked ? "markdown-body" : fallbackClass,
  });
  if (isMarked) {
    view.innerHTML = marked.parse(body);
  } else {
    view.textContent = body;
  }
  return el("div", { class: "field-block" }, [
    el("h4", { text: "body" }),
    view,
  ]);
}

function renderLocksPanel(payload) {
  const body = $("locks-body");
  const count = $("locks-count");
  if (!body || !count) return;
  const byTask = Array.isArray(payload && payload.by_task) ? payload.by_task : [];
  const totalLocked = Number.isFinite(Number(payload && payload.total_locked))
    ? Number(payload.total_locked)
    : 0;
  const totalTasks = Number.isFinite(Number(payload && payload.total_tasks))
    ? Number(payload.total_tasks)
    : byTask.length;
  count.textContent = `${totalLocked} files / ${totalTasks} tasks`;

  if (byTask.length === 0) {
    const empty = el("div", { class: "locks-empty", text: "No files currently locked." });
    empty.dataset.key = "locks-empty";
    empty.dataset.hash = "locks-empty";
    syncNodes(body, [empty]);
    return;
  }

  const nodes = byTask.map((task) => {
    const taskId = String(task.id || "");
    const group = el("div", { class: "lock-task-group" });
    group.dataset.key = `lock-task-${taskId}`;
    group.dataset.hash = JSON.stringify(task);

    const idButton = el("button", {
      class: "lock-task-id mono",
      text: `[${taskId}]`,
      title: `Open ${taskId} in the task list`,
    });
    idButton.addEventListener("click", (e) => {
      e.stopPropagation();
      openVisibleTask(taskId, taskContext());
    });

    const header = el("div", { class: "lock-task-header" }, [
      idButton,
      el("span", { class: "lock-separator", text: "·" }),
      statusPill(task.status || "unknown"),
    ]);
    if (task.job_run_id) {
      header.appendChild(el("span", { class: "lock-separator", text: "·" }));
      header.appendChild(el("span", {
        class: "lock-job mono",
        text: `job_run=${task.job_run_id}`,
        title: task.job_run_id,
      }));
    }
    group.appendChild(header);

    const files = Array.isArray(task.context_files) ? task.context_files : [];
    for (const path of files) {
      group.appendChild(el("div", {
        class: "lock-file-row mono",
        text: String(path),
        title: String(path),
      }));
    }
    return group;
  });
  syncNodes(body, nodes);
}

function fmtTimestamp(iso) {
  if (!iso) return "-";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const now = Date.now();
  const diff = (now - d.getTime()) / 1000;
  if (diff < 60) return `${Math.floor(diff)}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  return `${Math.floor(diff / 86400)}d`;
}

function fmtAbsTime(iso) {
  if (!iso) return "-";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fmtDuration(ms) {
  if (ms == null) return "-";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m${Math.floor((ms % 60000) / 1000)}s`;
}

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

const RUN_SORT_DEFAULT_DIR = {
  when: "desc",
  job: "asc",
  run_id: "asc",
  denials: "desc",
  tool_fails: "desc",
  duration: "desc",
  state: "asc",
};

function coerceNumber(value) {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string" || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function firstNumericField(row, keys) {
  for (const key of keys) {
    const value = coerceNumber(row && row[key]);
    if (value != null) return value;
  }
  return null;
}

function firstBooleanField(row, keys) {
  for (const key of keys) {
    const value = row && row[key];
    if (typeof value === "boolean") return value;
    if (typeof value === "string") {
      const normalized = value.trim().toLowerCase();
      if (normalized === "true") return true;
      if (normalized === "false") return false;
    }
  }
  return false;
}

function frictionRunId(row) {
  return (
    (row && (row.run_id || row.job_run || row.runId || row.jobRun)) ||
    ""
  );
}

function frictionRowText(row) {
  return [
    row && (row.kind || row.body_kind || row.event_kind || row.type || row.category),
    row && (row.command || row.tool || row.tool_name || row.status || row.outcome),
    row && (row.stderr || row.message || row.reason || row.error),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

function frictionRowLooksDenied(row) {
  const text = frictionRowText(row).replace(/[_-]/g, " ");
  return /\bden(?:y|ied|ial|ials)\b/.test(text) || text.includes("policy deny");
}

function frictionRowLooksToolFail(row) {
  const exitCode = coerceNumber(row && row.exit_code);
  if (exitCode != null && exitCode !== 0) return true;
  if (firstBooleanField(row, ["timed_out", "timeout"])) return true;
  const text = frictionRowText(row).replace(/[_-]/g, " ");
  return text.includes("tool") && /\b(fail|failed|failure|timeout|timed out)\b/.test(text);
}

function frictionRowLooksLongRun(row) {
  if (firstBooleanField(row, ["long_run", "is_long_run", "long_running"])) return true;
  const text = frictionRowText(row).replace(/[_-]/g, " ");
  return text.includes("long run") || text.includes("long running");
}

function emptyRunFrictionSummary(run) {
  return {
    denials: 0,
    toolFails: 0,
    durationMs: coerceNumber(run && run.duration_ms),
    longRun: false,
  };
}

function addFrictionRowToSummary(summary, row) {
  const denials = firstNumericField(row, [
    "denials",
    "denial_count",
    "policy_denials",
  ]);
  if (denials != null) {
    summary.denials += denials;
  } else if (frictionRowLooksDenied(row)) {
    summary.denials += 1;
  }

  const toolFails = firstNumericField(row, [
    "tool_fails",
    "tool_failures",
    "tool_fail_count",
    "failed_tool_calls",
  ]);
  if (toolFails != null) {
    summary.toolFails += toolFails;
  } else if (frictionRowLooksToolFail(row)) {
    summary.toolFails += 1;
  }

  const durationMs = firstNumericField(row, [
    "duration_ms",
    "run_duration_ms",
    "wall_clock_ms",
    "elapsed_ms",
  ]);
  if (durationMs != null) {
    summary.durationMs = Math.max(summary.durationMs || 0, durationMs);
  }
  summary.longRun = summary.longRun || frictionRowLooksLongRun(row);
}

function mergeRunsWithFriction(runs, frictionRows) {
  const byRun = new Map();
  for (const row of frictionRows || []) {
    const runId = frictionRunId(row);
    if (!runId) continue;
    if (!byRun.has(runId)) byRun.set(runId, emptyRunFrictionSummary());
    addFrictionRowToSummary(byRun.get(runId), row);
  }
  return (runs || []).map((run) => ({
    ...run,
    diagnostics_friction: mergeRunFrictionSummary(run, byRun.get(run.run_id)),
  }));
}

function mergeRunFrictionSummary(run, friction) {
  const base = emptyRunFrictionSummary(run);
  if (!friction) return base;
  return {
    ...base,
    ...friction,
    durationMs: friction.durationMs == null ? base.durationMs : friction.durationMs,
  };
}

function runTimestampValue(run) {
  const ts = run.finished_at || run.started_at || run.scheduled_at || run.created_at;
  const time = ts ? new Date(ts).getTime() : 0;
  return Number.isFinite(time) ? time : 0;
}

function runFriction(run) {
  return run.diagnostics_friction || emptyRunFrictionSummary(run);
}

function runSortValue(run, key) {
  const friction = runFriction(run);
  switch (key) {
    case "when":
      return runTimestampValue(run);
    case "job":
      return run.job_id || "";
    case "run_id":
      return run.run_id || "";
    case "denials":
      return friction.denials || 0;
    case "tool_fails":
      return friction.toolFails || 0;
    case "duration":
      return friction.durationMs || 0;
    case "state":
      return run.state || "";
    default:
      return "";
  }
}

function compareRunValues(left, right) {
  if (typeof left === "number" && typeof right === "number") return left - right;
  return String(left).localeCompare(String(right));
}

function sortedRunsForDisplay(runs) {
  const rows = (runs || []).slice();
  rows.sort((a, b) => {
    const primary = compareRunValues(runSortValue(a, runSort.key), runSortValue(b, runSort.key));
    const directed = runSort.dir === "asc" ? primary : -primary;
    if (directed !== 0) return directed;
    return runTimestampValue(b) - runTimestampValue(a);
  });
  return rows;
}

function setRunSort(key) {
  if (runSort.key === key) {
    runSort = { key, dir: runSort.dir === "asc" ? "desc" : "asc" };
  } else {
    runSort = { key, dir: RUN_SORT_DEFAULT_DIR[key] || "asc" };
  }
  renderRuns(lastRuns);
}

function runHeaderCell(label, key, opts = {}) {
  const classes = [opts.class, opts.num ? "num" : ""].filter(Boolean).join(" ");
  const cell = el("span", { class: classes, style: opts.style });
  const button = el("button", {
    class: `runs-sort${runSort.key === key ? " active" : ""}`,
    text: label,
    title: `Sort Recent Runs by ${label}`,
  });
  button.type = "button";
  if (runSort.key === key) {
    button.appendChild(el("span", {
      class: "sort-arrow",
      text: runSort.dir === "asc" ? " ▲" : " ▼",
    }));
  }
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    setRunSort(key);
  });
  cell.appendChild(button);
  return cell;
}

function runCountCell(value) {
  return el("span", {
    class: `num${value > 0 ? " hot" : ""}`,
    text: String(value || 0),
  });
}

function runDurationCell(run) {
  const friction = runFriction(run);
  return el("span", { class: "duration" }, [
    fmtDuration(friction.durationMs),
    friction.longRun
      ? el("span", { class: "long-run-flag", text: "!", title: "Long run" })
      : null,
  ]);
}

function renderRuns(runs) {
  const body = $("runs-body");
  const frag = document.createDocumentFragment();
  
  const sorted = sortedRunsForDisplay(runs);
  const top = sorted.slice(0, 20);
  if ($("diag-count") && activeDiagSubtab === "runs") {
    $("diag-count").textContent = `${top.length}/${sorted.length}`;
  }
  if (top.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No job runs yet." })
    ])]);
    return;
  }
  const header = el("div", { class: "runs-row runs-header" }, [
    runHeaderCell("when", "when"),
    runHeaderCell("job", "job"),
    runHeaderCell("run id", "run_id"),
    runHeaderCell("denials", "denials", { num: true }),
    runHeaderCell("tool fails", "tool_fails", { num: true }),
    runHeaderCell("duration", "duration", { style: { textAlign: "right" } }),
    runHeaderCell("state", "state", { style: { textAlign: "right" } }),
    el("span", { text: "" }),
  ]);
  header.dataset.key = "runs-header";
  header.dataset.hash = `header-${runSort.key}-${runSort.dir}`;
  frag.appendChild(header);
  for (const r of top) {
    const ts = r.finished_at || r.started_at || r.scheduled_at || r.created_at;
    const friction = runFriction(r);
    const runIdSpan = el("span", { class: "run-id", text: r.run_id, title: "Click to copy run ID" });
    runIdSpan.addEventListener("click", (e) => {
      e.stopPropagation();
      navigator.clipboard.writeText(r.run_id).catch(() => {});
      const oldText = runIdSpan.textContent;
      runIdSpan.textContent = "copied!";
      runIdSpan.style.color = "var(--state-success)";
      setTimeout(() => {
        runIdSpan.textContent = oldText;
        runIdSpan.style.color = "";
      }, 1000);
    });
    const row = el("div", { class: "runs-row", title: `${r.run_id} (click to inspect)` }, [
      el("span", { class: "when", text: fmtTimestamp(ts) }),
      el("span", { class: "id", text: r.job_id }),
      runIdSpan,
      runCountCell(friction.denials),
      runCountCell(friction.toolFails),
      runDurationCell(r),
      el("span", { class: "state" }, [stateCell(r.state)]),
      el("span", { class: "run-actions" }, runIsCancellable(r) ? [buildCancelRunButton(r, body)] : []),
    ]);
    row.dataset.key = `run-${r.run_id}`;
    row.dataset.hash = `${r.run_id}-${ts}-${r.duration_ms}-${r.state}-${friction.denials}-${friction.toolFails}-${friction.durationMs}-${friction.longRun}`;
    row.style.cursor = "pointer";
    row.addEventListener("click", () => navigateToRun(r.run_id));
    frag.appendChild(row);
  }
  syncNodes(body, Array.from(frag.children));
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
    th.addEventListener("click", () => navigateToRole(name, auditContext()));
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
    label.addEventListener("click", () => navigateToRole(family, auditContext()));
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

function evidenceCount(learning) {
  return Array.isArray(learning && learning.evidence) ? learning.evidence.length : 0;
}

function learningScopeNodes(learning) {
  const scope = (learning && learning.scope) || {};
  const paths = Array.isArray(scope.paths) ? scope.paths : [];
  const tags = Array.isArray(scope.tags) ? scope.tags : [];
  const chips = [];
  for (const tag of tags.slice(0, 3)) {
    chips.push(el("span", { class: "pill", text: `#${tag}`, title: tag }));
  }
  for (const path of paths.slice(0, Math.max(0, 4 - chips.length))) {
    chips.push(el("span", { class: "pill", text: truncate(path, 28), title: path }));
  }
  if (paths.length + tags.length > chips.length) {
    chips.push(el("span", { class: "pill", text: `+${paths.length + tags.length - chips.length}` }));
  }
  if (chips.length === 0) chips.push(el("span", { class: "pill", text: "global" }));
  return chips;
}

function renderLearningStats(stats = {}) {
  $("learning-total-value").textContent = formatBigInt(stats.total || 0);
  $("learning-superseded-value").textContent = formatBigInt(stats.superseded || 0);
  $("learning-last-indexed-value").textContent = stats.last_indexed
    ? fmtTimestamp(stats.last_indexed)
    : "-";
}

function renderLearnings(payload) {
  const body = $("learnings-body");
  if (!body) return;
  const items = Array.isArray(payload && payload.items) ? payload.items : [];
  const stats = (payload && payload.stats) || {};
  renderLearningStats(stats);
  $("knowledge-count").textContent = `${items.length}/${stats.total || items.length}`;

  if (items.length > 0 && !items.some((item) => item.id === activeLearningId)) {
    activeLearningId = items[0].id;
  }
  if (items.length === 0) activeLearningId = null;

  if (items.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No learnings match the current filter." }),
    ])]);
    renderLearningDetail(null);
    return;
  }

  const frag = document.createDocumentFragment();
  const header = el("div", { class: "learning-row header" }, [
    el("span", { text: "id" }),
    el("span", { text: "scope" }),
    el("span", { class: "evidence", text: "evidence" }),
    el("span", { text: "status" }),
    el("span", { class: "updated", text: "updated" }),
  ]);
  header.dataset.key = "learning-header";
  header.dataset.hash = "learning-header";
  frag.appendChild(header);

  for (const learning of items) {
    const row = el("div", { class: "learning-row", title: learning.summary || learning.id }, [
      el("span", { class: "id", text: learning.id, title: learning.id }),
      el("span", { class: "scope" }, learningScopeNodes(learning)),
      el("span", { class: "evidence", text: String(evidenceCount(learning)) }),
      statusPill(learning.status || "active"),
      el("span", { class: "updated", text: fmtTimestamp(learning.updated_at), title: fmtAbsTime(learning.updated_at) }),
    ]);
    row.dataset.key = `learning-${learning.id}`;
    row.dataset.hash = `${learning.id}-${learning.status}-${learning.updated_at}-${activeLearningId === learning.id}`;
    if (activeLearningId === learning.id) row.classList.add("active");
    row.addEventListener("click", () => {
      activeLearningId = learning.id;
      renderLearnings(lastLearningPayload);
    });
    frag.appendChild(row);
  }

  syncNodes(body, Array.from(frag.children));
  renderLearningDetail(items.find((item) => item.id === activeLearningId) || items[0]);
}

function renderLearningDetail(learning) {
  const detail = $("learning-detail");
  if (!detail) return;
  const count = $("learning-detail-count");
  if (!learning) {
    if (count) count.textContent = "-";
    syncNodes(detail, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No learning selected." }),
    ])]);
    return;
  }
  if (count) count.textContent = learning.status || "active";

  const title = el("div", { class: "field-block" }, [
    el("h4", { text: learning.id }),
    el("div", { class: "markdown-body", text: learning.summary || "" }),
  ]);

  const meta = el("div", { class: "learning-detail-meta" });
  const addMeta = (label, value) => {
    if (value == null || value === "") return;
    meta.appendChild(el("span", {}, [
      document.createTextNode(`${label}: `),
      el("span", { class: "value", text: String(value) }),
    ]));
  };
  addMeta("status", learning.status || "active");
  addMeta("evidence", evidenceCount(learning));
  addMeta("updated", fmtAbsTime(learning.updated_at));
  addMeta("superseded_by", learning.superseded_by);
  addMeta("supersedes", learning.supersedes);

  const scopeBlock = el("div", { class: "field-block" }, [
    el("h4", { text: "scope" }),
    el("div", { class: "learning-detail-scope" }, learningScopeNodes(learning)),
  ]);

  const bodyBlock = renderBodyBlock(learning.body, "learning-detail-body");

  const actions = el("div", { class: "actions" });
  const supersede = el("button", {
    class: "action archive",
    text: "Supersede",
    title: `Supersede ${learning.id}`,
  });
  supersede.disabled = learning.status === "superseded";
  supersede.addEventListener("click", () => {
    const by = window.prompt(`Replacement learning ID for ${learning.id}`);
    if (!by || !by.trim()) return;
    supersedeLearning(learning, by.trim(), supersede, detail);
  });
  actions.appendChild(supersede);

  const nodes = [title, meta, scopeBlock];
  if (bodyBlock) nodes.push(bodyBlock);
  nodes.push(actions);
  syncNodes(detail, nodes);
}

async function supersedeLearning(learning, by, btn, detail) {
  const oldText = btn.textContent;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>wait`;
  for (const node of detail.querySelectorAll(".action-error")) node.remove();
  try {
    await postJson(`/api/learnings/${encodeURIComponent(learning.id)}/supersede`, { by });
    activeLearningId = learning.id;
    await fetchAndRenderLearnings();
  } catch (e) {
    detail.prepend(el("div", { class: "action-error", text: e.message || "supersede failed" }));
  } finally {
    btn.disabled = false;
    btn.textContent = oldText;
  }
}

function frictionTagNodes(tags = []) {
  const values = Array.isArray(tags) ? tags : [];
  if (values.length === 0) return [el("span", { class: "pill", text: "-" })];
  return values.map((tag) => el("span", { class: "pill mono", text: tag, title: tag }));
}

function renderFrictionStats(stats = {}) {
  $("friction-open-value").textContent = formatBigInt(stats.open || 0);
  $("friction-triaged-value").textContent = formatBigInt(stats.triaged || 0);
  $("friction-resolved-month-value").textContent = formatBigInt(stats.resolved_this_month || 0);
}

function renderFrictions(payload) {
  const body = $("frictions-body");
  if (!body) return;
  const items = Array.isArray(payload && payload.items) ? payload.items : [];
  const stats = (payload && payload.stats) || {};
  renderFrictionStats(stats);
  $("knowledge-count").textContent = `${items.length}/${stats.total || items.length}`;

  if (items.length > 0 && !items.some((item) => item.id === activeFrictionId)) {
    activeFrictionId = items[0].id;
  }
  if (items.length === 0) activeFrictionId = null;

  if (items.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No frictions match the current filter." }),
    ])]);
    renderFrictionDetail(null);
    return;
  }

  const frag = document.createDocumentFragment();
  const header = el("div", { class: "friction-row header" }, [
    el("span", { text: "id" }),
    el("span", { text: "title" }),
    el("span", { class: "tags", text: "tags" }),
    el("span", { text: "status" }),
    el("span", { class: "reported", text: "reported" }),
  ]);
  header.dataset.key = "friction-header";
  header.dataset.hash = "friction-header";
  frag.appendChild(header);

  for (const friction of items) {
    const title = friction.title || friction.id;
    const row = el("div", { class: "friction-row", title }, [
      el("span", { class: "id", text: friction.id, title: friction.id }),
      el("span", { class: "title", text: title }),
      el("span", { class: "tags" }, frictionTagNodes(friction.tags)),
      statusPill(friction.status || "open"),
      el("span", { class: "reported", text: fmtTimestamp(friction.created_at), title: fmtAbsTime(friction.created_at) }),
    ]);
    row.dataset.key = `friction-${friction.id}`;
    row.dataset.hash = `${friction.id}-${friction.status}-${(friction.tags || []).join(",")}-${friction.created_at}-${activeFrictionId === friction.id}`;
    if (activeFrictionId === friction.id) row.classList.add("active");
    row.addEventListener("click", () => {
      activeFrictionId = friction.id;
      renderFrictions(lastFrictionPayload);
    });
    frag.appendChild(row);
  }

  syncNodes(body, Array.from(frag.children));
  renderFrictionDetail(items.find((item) => item.id === activeFrictionId) || items[0]);
}

function renderFrictionDetail(friction) {
  const detail = $("friction-detail");
  if (!detail) return;
  const count = $("friction-detail-count");
  if (!friction) {
    if (count) count.textContent = "-";
    syncNodes(detail, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No friction selected." }),
    ])]);
    return;
  }
  if (count) count.textContent = friction.status || "open";

  const title = el("div", { class: "field-block" }, [
    el("h4", { text: friction.id }),
    el("div", { class: "markdown-body", text: friction.title || "" }),
  ]);

  const meta = el("div", { class: "friction-detail-meta" });
  const addMeta = (label, value) => {
    if (value == null || value === "") return;
    meta.appendChild(el("span", {}, [
      document.createTextNode(`${label}: `),
      el("span", { class: "value", text: String(value) }),
    ]));
  };
  addMeta("status", friction.status || "open");
  addMeta("model", friction.model);
  addMeta("reported", fmtAbsTime(friction.created_at));
  addMeta("resolved", friction.resolved_at ? fmtAbsTime(friction.resolved_at) : null);
  addMeta("task", friction.during_task);

  const controls = el("div", { class: "field-block friction-controls" }, [
    el("h4", { text: "triage" }),
  ]);
  const controlGrid = el("div", { class: "friction-control-grid" });
  controlGrid.appendChild(buildFrictionStatusControl(friction, detail));
  controlGrid.appendChild(buildFrictionTagPicker(friction, detail));
  controls.appendChild(controlGrid);

  const bodyBlock = el("div", { class: "field-block" }, [
    el("h4", { text: "body" }),
    el("pre", { class: "friction-detail-body", text: friction.body || "" }),
  ]);

  const actions = el("div", { class: "actions" });
  const resolve = el("button", {
    class: "action approve",
    text: "Resolve",
    title: `Resolve ${friction.id}`,
  });
  resolve.disabled = friction.status === "resolved";
  resolve.addEventListener("click", () => resolveFriction(friction, resolve, detail));
  actions.appendChild(resolve);

  syncNodes(detail, [title, meta, controls, bodyBlock, actions]);
}

function buildFrictionStatusControl(friction, detail) {
  const wrap = el("label", { class: "friction-control" });
  wrap.appendChild(el("span", { class: "friction-control-label", text: "status" }));
  const select = el("select", { class: "action status-update", title: `Status for ${friction.id}` });
  for (const status of FRICTION_STATUSES) {
    const option = el("option", { text: status });
    option.value = status;
    option.selected = (friction.status || "open") === status;
    select.appendChild(option);
  }
  select.addEventListener("change", () => {
    patchFriction(friction, { status: select.value }, select, detail);
  });
  wrap.appendChild(select);
  return wrap;
}

function buildFrictionTagPicker(friction, detail) {
  const wrap = el("div", { class: "friction-control friction-tag-picker" }, [
    el("span", { class: "friction-control-label", text: "tags" }),
  ]);
  const options = Array.isArray(lastFrictionPayload.tags) ? lastFrictionPayload.tags : [];
  const selected = new Set(Array.isArray(friction.tags) ? friction.tags : []);
  const grid = el("div", { class: "friction-tag-options" });
  const checkboxes = new Map();
  if (options.length === 0) {
    grid.appendChild(el("span", { class: "pill", text: "-" }));
  }
  for (const tag of options) {
    const id = `friction-tag-${friction.id}-${tag}`;
    const checkbox = el("input");
    checkbox.type = "checkbox";
    checkbox.id = id;
    checkbox.checked = selected.has(tag);
    checkboxes.set(tag, checkbox);
    checkbox.addEventListener("change", () => {
      const tags = options.filter((option) => checkboxes.get(option)?.checked);
      if (tags.length === 0) {
        checkbox.checked = true;
        return;
      }
      patchFriction(friction, { tags }, checkbox, detail);
    });
    const label = el("label", { class: "friction-tag-option", title: tag }, [
      checkbox,
      el("span", { text: tag }),
    ]);
    grid.appendChild(label);
  }
  wrap.appendChild(grid);
  return wrap;
}

async function patchFriction(friction, patch, control, detail) {
  if (!friction || !friction.id) return;
  if (control) control.disabled = true;
  for (const node of detail.querySelectorAll(".action-error")) node.remove();
  try {
    const updated = await patchJson(`/api/frictions/${encodeURIComponent(friction.id)}`, patch);
    activeFrictionId = updated.id || friction.id;
    await fetchAndRenderFrictions();
  } catch (e) {
    detail.prepend(el("div", { class: "action-error", text: e.message || "friction update failed" }));
    if (patch.status && control) control.value = friction.status || "open";
  } finally {
    if (control) control.disabled = false;
  }
}

async function resolveFriction(friction, btn, detail) {
  const oldText = btn.textContent;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>wait`;
  for (const node of detail.querySelectorAll(".action-error")) node.remove();
  try {
    const updated = await postJson(`/api/frictions/${encodeURIComponent(friction.id)}/resolve`);
    activeFrictionId = updated.id || friction.id;
    await fetchAndRenderFrictions();
  } catch (e) {
    detail.prepend(el("div", { class: "action-error", text: e.message || "resolve failed" }));
  } finally {
    btn.disabled = false;
    btn.textContent = oldText;
  }
}

function adrList(adr, field) {
  return Array.isArray(adr && adr[field]) ? adr[field] : [];
}

function adrPrimaryFeature(adr) {
  const features = adrList(adr, "related_features");
  if (features.length === 0) return "-";
  if (features.length === 1) return features[0];
  return `${features[0]} +${features.length - 1}`;
}

function renderAdrStats(stats = {}) {
  $("adr-proposed-value").textContent = formatBigInt(stats.proposed || 0);
  $("adr-accepted-value").textContent = formatBigInt(stats.accepted || 0);
  $("adr-superseded-value").textContent = formatBigInt(stats.superseded || 0);
}

function renderAdrs(payload) {
  const body = $("adrs-body");
  if (!body) return;
  const items = Array.isArray(payload && payload.items) ? payload.items : [];
  const stats = (payload && payload.stats) || {};
  renderAdrStats(stats);
  $("knowledge-count").textContent = `${items.length}/${stats.total || items.length}`;

  if (items.length > 0 && !items.some((item) => item.id === activeAdrId)) {
    activeAdrId = items[0].id;
  }
  if (items.length === 0) activeAdrId = null;

  if (items.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No ADRs match the current filter." }),
    ])]);
    renderAdrDetail(null);
    return;
  }

  const frag = document.createDocumentFragment();
  const header = el("div", { class: "adr-row header" }, [
    el("span", { text: "id" }),
    el("span", { text: "title" }),
    el("span", { text: "status" }),
    el("span", { class: "feature", text: "feature" }),
    el("span", { class: "accepted", text: "accepted-at" }),
  ]);
  header.dataset.key = "adr-header";
  header.dataset.hash = "adr-header";
  frag.appendChild(header);

  for (const adr of items) {
    const feature = adrPrimaryFeature(adr);
    const accepted = adr.accepted_at ? fmtTimestamp(adr.accepted_at) : "-";
    const row = el("div", { class: "adr-row", title: adr.title || adr.id }, [
      el("span", { class: "id", text: adr.id, title: adr.id }),
      el("span", { class: "title", text: adr.title || "" }),
      statusPill(adr.status || "proposed"),
      el("span", { class: "feature", text: feature, title: adrList(adr, "related_features").join(", ") }),
      el("span", { class: "accepted", text: accepted, title: fmtAbsTime(adr.accepted_at) }),
    ]);
    row.dataset.key = `adr-${adr.id}`;
    row.dataset.hash = `${adr.id}-${adr.status}-${adr.accepted_at || ""}-${adr.superseded_by || ""}-${activeAdrId === adr.id}`;
    if (activeAdrId === adr.id) row.classList.add("active");
    row.addEventListener("click", () => {
      activeAdrId = adr.id;
      renderAdrs(lastAdrPayload);
    });
    frag.appendChild(row);
  }

  syncNodes(body, Array.from(frag.children));
  renderAdrDetail(items.find((item) => item.id === activeAdrId) || items[0]);
}

function buildAdrValueList(values, opts = {}) {
  const wrap = el("div", { class: "adr-detail-list" });
  if (!values || values.length === 0) {
    wrap.appendChild(el("span", { class: "pill", text: "-" }));
    return wrap;
  }
  for (const value of values) {
    if (opts.taskLinks) {
      const btn = el("button", { class: "relation-target mono", text: value, title: `Open task ${value}` });
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        openTaskFromKnowledge(value);
      });
      wrap.appendChild(btn);
    } else {
      wrap.appendChild(el("span", { class: "pill mono", text: value, title: value }));
    }
  }
  return wrap;
}

function openTaskFromKnowledge(taskId) {
  activeStatuses = new Set(STATUS_ORDER);
  searchQuery = "";
  const taskSearch = $("task-search");
  if (taskSearch) taskSearch.value = "";
  setActiveTab("tasks", { refresh: false });
  const open = () => openVisibleTask(taskId, taskContext());
  if (lastTasks.length > 0 && hasCrewOptions()) {
    open();
    return;
  }
  fetchAndRenderTasks().then(() => {
    open();
  }).catch(() => copyTaskIdWithNotice(taskId, taskContext()));
}

function renderAdrDetail(adr) {
  const detail = $("adr-detail");
  if (!detail) return;
  const count = $("adr-detail-count");
  if (!adr) {
    if (count) count.textContent = "-";
    syncNodes(detail, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No ADR selected." }),
    ])]);
    return;
  }
  if (count) count.textContent = adr.status || "proposed";

  const title = el("div", { class: "field-block" }, [
    el("h4", { text: adr.id }),
    el("div", { class: "markdown-body", text: adr.title || "" }),
  ]);

  const meta = el("div", { class: "adr-detail-meta" });
  const addMeta = (label, value) => {
    if (value == null || value === "") return;
    meta.appendChild(el("span", {}, [
      document.createTextNode(`${label}: `),
      el("span", { class: "value", text: String(value) }),
    ]));
  };
  addMeta("status", adr.status || "proposed");
  addMeta("owner", adr.owner);
  addMeta("created", fmtAbsTime(adr.created_at));
  addMeta("accepted", fmtAbsTime(adr.accepted_at));
  addMeta("updated", fmtAbsTime(adr.last_updated));

  const featuresBlock = el("div", { class: "field-block" }, [
    el("h4", { text: "related_features" }),
    buildAdrValueList(adrList(adr, "related_features")),
  ]);
  const tasksBlock = el("div", { class: "field-block" }, [
    el("h4", { text: "related_tasks" }),
    buildAdrValueList(adrList(adr, "related_tasks"), { taskLinks: true }),
  ]);
  const edgesBlock = el("div", { class: "field-block" }, [
    el("h4", { text: "supersession" }),
    buildAdrValueList([
      ...adrList(adr, "supersedes").map((id) => `supersedes ${id}`),
      ...(adr.superseded_by ? [`superseded_by ${adr.superseded_by}`] : []),
    ]),
  ]);
  const bodyBlock = renderBodyBlock(adr.body, "adr-detail-body");

  const actions = el("div", { class: "actions" });
  if (adr.status === "proposed") {
    const accept = el("button", {
      class: "action approve",
      text: "Accept",
      title: `Accept ${adr.id}`,
    });
    accept.addEventListener("click", () => acceptAdr(adr, accept, detail));
    actions.appendChild(accept);
  }
  if (adr.status === "accepted") {
    const supersede = el("button", {
      class: "action archive",
      text: "Supersede",
      title: `Supersede ${adr.id}`,
    });
    supersede.addEventListener("click", () => {
      const by = window.prompt(`Replacement ADR ID for ${adr.id}`);
      if (!by || !by.trim()) return;
      supersedeAdr(adr, by.trim(), supersede, detail);
    });
    actions.appendChild(supersede);
  }

  const nodes = [title, meta, featuresBlock, tasksBlock, edgesBlock];
  if (bodyBlock) nodes.push(bodyBlock);
  if (actions.children.length > 0) nodes.push(actions);
  syncNodes(detail, nodes);
}

async function acceptAdr(adr, btn, detail) {
  await runAdrAction(
    adr,
    btn,
    detail,
    () => postJson(`/api/adrs/${encodeURIComponent(adr.id)}/accept`),
    "accept failed",
  );
}

async function supersedeAdr(adr, by, btn, detail) {
  await runAdrAction(
    adr,
    btn,
    detail,
    () => postJson(`/api/adrs/${encodeURIComponent(adr.id)}/supersede`, { by }),
    "supersede failed",
  );
}

async function runAdrAction(adr, btn, detail, action, fallbackMessage) {
  const oldText = btn.textContent;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>wait`;
  for (const node of detail.querySelectorAll(".action-error")) node.remove();
  try {
    await action();
    activeAdrId = adr.id;
    await fetchAndRenderAdrs();
  } catch (e) {
    detail.prepend(el("div", { class: "action-error", text: e.message || fallbackMessage }));
  } finally {
    btn.disabled = false;
    btn.textContent = oldText;
  }
}

function wireLearningSearch() {
  const input = $("learning-search");
  if (!input) return;
  let debounce = null;
  input.addEventListener("input", (e) => {
    learningSearchQuery = e.target.value.trim();
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => {
      if (activeTab === "knowledge") fetchAndRenderLearnings().catch(console.error);
    }, 200);
  });
}

function wireAdrSearch() {
  const input = $("adr-search");
  if (!input) return;
  let debounce = null;
  input.addEventListener("input", (e) => {
    adrSearchQuery = e.target.value.trim();
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => {
      if (activeTab === "knowledge") fetchAndRenderAdrs().catch(console.error);
    }, 200);
  });
}

function wireFrictionSearch() {
  const input = $("friction-search");
  if (!input) return;
  let debounce = null;
  input.addEventListener("input", (e) => {
    frictionSearchQuery = e.target.value.trim();
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => {
      if (activeTab === "knowledge") fetchAndRenderFrictions().catch(console.error);
    }, 200);
  });
}

const TABS = ["tasks", "scoreboard", "audit", "diagnostics", "knowledge", "run-detail"];
const DIAG_SUBTABS = ["runs", "metrics", "errors"];
const RUN_DETAIL_SUBTABS = ["steps", "events"];
const KNOWLEDGE_SUBTABS = ["learnings", "adrs", "frictions"];

function parseHashRoute(raw) {
  const trimmed = String(raw || "").replace(/^#/, "");
  const queryIdx = trimmed.indexOf("?");
  const path = queryIdx >= 0 ? trimmed.slice(0, queryIdx) : trimmed;
  const queryStr = queryIdx >= 0 ? trimmed.slice(queryIdx + 1) : "";
  const segments = path.split("/").filter(Boolean);
  const query = new URLSearchParams(queryStr);
  return { segments, query };
}

function setActiveTab(raw, opts = {}) {
  const { segments, query } = parseHashRoute(raw);
  const head = segments[0] || "tasks";
  if (head === "runs" && !segments[1] && query.get("run_id")) {
    segments[1] = encodeURIComponent(query.get("run_id"));
  }
  let top;
  if (head === "runs" && segments[1]) {
    top = "run-detail";
    const nextRunId = decodeURIComponent(segments[1]);
    if (activeRunId !== nextRunId) {
      activeRunLogs = [];
      expandedStepIndices.clear();
    }
    activeRunId = nextRunId;
    const expandStep = query.get("step");
    if (expandStep != null && /^\d+$/.test(expandStep)) {
      expandedStepIndices.add(Number(expandStep));
    }
    const sub = RUN_DETAIL_SUBTABS.includes(segments[2]) ? segments[2] : activeRunSubtab;
    activeRunSubtab = sub;
  } else if (TABS.includes(head)) {
    top = head;
  } else {
    top = "tasks";
  }
  activeTab = top;
  for (const tab of document.querySelectorAll(".tab")) {
    tab.classList.toggle("active", tab.dataset.tab === top);
  }
  for (const pane of document.querySelectorAll(".tab-pane")) {
    pane.classList.toggle("active", pane.dataset.tab === top);
  }
  if (top === "tasks") requestAnimationFrame(fitLogPanelToViewport);

  const indicator = $("tab-indicator") || el("div", {id: "tab-indicator", class: "tab-indicator"});
  if (!indicator.parentNode) document.querySelector(".tabs").appendChild(indicator);
  // For run-detail (no top tab button), hide the indicator
  const activeTabEl = document.querySelector(`.tab[data-tab="${top}"]`);
  if (activeTabEl) {
    indicator.style.display = "";
    indicator.style.width = `${activeTabEl.offsetWidth}px`;
    indicator.style.left = `${activeTabEl.offsetLeft}px`;
  } else {
    indicator.style.display = "none";
  }

  let hash;
  if (top === "diagnostics") {
    const sub = DIAG_SUBTABS.includes(segments[1]) ? segments[1] : activeDiagSubtab;
    setDiagSubtab(sub);
    hash = `#diagnostics/${sub}`;
  } else if (top === "audit") {
    applyAuditHashQuery(query);
    const sub = ["events", "policy"].includes(segments[1]) ? segments[1] : getActiveAuditSubtab();
    setAuditSubtab(sub);
    hash = buildAuditHash();
    syncAuditControls();
  } else if (top === "run-detail") {
    setRunDetailSubtab(activeRunSubtab);
    hash = `#runs/${encodeURIComponent(activeRunId || "")}` +
      (activeRunSubtab !== "steps" ? `/${activeRunSubtab}` : "");
    if (query.get("step") != null) hash += `?step=${encodeURIComponent(query.get("step"))}`;
  } else if (top === "knowledge") {
    const sub = KNOWLEDGE_SUBTABS.includes(segments[1]) ? segments[1] : activeKnowledgeSubtab;
    setKnowledgeSubtab(sub);
    hash = sub === "learnings" ? "#knowledge/learnings" : `#knowledge/${sub}`;
  } else {
    hash = `#${top}`;
  }
  const hashChanged = window.location.hash !== hash;
  const shouldUpdateHash = opts.updateHash !== false;
  if (hashChanged && shouldUpdateHash) {
    window.location.hash = hash;
  }
  if (opts.refresh !== false && (!hashChanged || !shouldUpdateHash)) refreshDashboard();
}

function setRunDetailSubtab(name) {
  if (!RUN_DETAIL_SUBTABS.includes(name)) name = "steps";
  activeRunSubtab = name;
  for (const btn of document.querySelectorAll("#run-detail-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }
  $("run-steps-body").style.display = name === "steps" ? "block" : "none";
  $("run-events-body").style.display = name === "events" ? "block" : "none";
}

function setDiagSubtab(name) {
  if (!DIAG_SUBTABS.includes(name)) name = "runs";
  activeDiagSubtab = name;
  for (const btn of document.querySelectorAll("#diag-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }

  const subIndicator = $("subtab-indicator") || el("div", {id: "subtab-indicator", class: "tab-indicator"});
  if (!subIndicator.parentNode) document.querySelector("#diag-subtabs").appendChild(subIndicator);
  const activeBtn = document.querySelector(`.subtab[data-subtab="${name}"]`);
  if (activeBtn) {
    subIndicator.style.width = `${activeBtn.offsetWidth}px`;
    subIndicator.style.left = `${activeBtn.offsetLeft}px`;
  }

  if (name === "runs") {
    $("diag-body").style.display = "none";
    $("runs-body").style.display = "block";
    renderRuns(lastRuns);
  } else {
    $("diag-body").style.display = "block";
    $("runs-body").style.display = "none";
    renderDiagnostics(diagnosticsContext());
  }
}

function setKnowledgeSubtab(name) {
  if (!KNOWLEDGE_SUBTABS.includes(name)) name = "learnings";
  activeKnowledgeSubtab = name;
  for (const btn of document.querySelectorAll("#knowledge-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }
  const isAdrs = name === "adrs";
  const isFrictions = name === "frictions";
  const isLearnings = name === "learnings";
  const toggle = (id, show) => {
    const node = $(id);
    if (node) node.style.display = show ? "" : "none";
  };
  toggle("learning-stats", isLearnings);
  toggle("learning-search", isLearnings);
  toggle("learnings-body", isLearnings);
  toggle("learning-detail-panel", isLearnings);
  toggle("adr-stats", isAdrs);
  toggle("adr-search", isAdrs);
  toggle("adrs-body", isAdrs);
  toggle("adr-detail-panel", isAdrs);
  toggle("friction-stats", isFrictions);
  toggle("friction-search", isFrictions);
  toggle("frictions-body", isFrictions);
  toggle("friction-detail-panel", isFrictions);
}

function initTabs() {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.addEventListener("click", () => setActiveTab(tab.dataset.tab, { refresh: false }));
  }
  for (const btn of document.querySelectorAll("#diag-subtabs .subtab")) {
    btn.addEventListener("click", () =>
      setActiveTab(`diagnostics/${btn.dataset.subtab}`, { refresh: false }),
    );
  }
  for (const btn of document.querySelectorAll("#run-detail-subtabs .subtab")) {
    btn.addEventListener("click", () => {
      activeRunSubtab = btn.dataset.subtab;
      const path = `runs/${encodeURIComponent(activeRunId || "")}` +
        (activeRunSubtab !== "steps" ? `/${activeRunSubtab}` : "");
      setActiveTab(path, { refresh: false });
      refreshDashboard();
    });
  }
  for (const btn of document.querySelectorAll("#audit-subtabs .subtab")) {
    btn.addEventListener("click", () => {
      setActiveAuditSubtabFromButton(btn.dataset.subtab);
      const newHash = buildAuditHash();
      if (window.location.hash !== newHash) {
        window.location.hash = newHash;
      } else {
        refreshDashboard();
      }
    });
  }
  for (const btn of document.querySelectorAll("#knowledge-subtabs .subtab")) {
    btn.addEventListener("click", () =>
      setActiveTab(`knowledge/${btn.dataset.subtab}`, { refresh: false }),
    );
  }
  window.addEventListener("hashchange", () => {
    setActiveTab(window.location.hash);
  });
  setActiveTab(window.location.hash || "tasks", {
    refresh: false,
    updateHash: false,
  });
  refreshDashboard();
  setInterval(refreshDashboard, 30000);
}

function fmtRelative(iso) {
  return fmtTimestamp(iso);
}

function truncate(text, max) {
  if (text == null) return "";
  if (text.length <= max) return text;
  return text.slice(0, max) + "\u2026";
}

function fetchAndCacheCrews() {
  return fetchJson("/api/crews").then((payload) => {
    return cacheCrewPayload(payload);
  });
}

function fetchAndRenderTasks() {
  return Promise.all([
    fetchJson("/api/tasks"),
    fetchAndCacheCrews(),
  ]).then(([tasks]) => {
    lastTasks = tasks;
    renderTasks(tasks, taskContext());
  });
}

function activeRefreshJobs() {
  // The health strip is global; refresh on every tick alongside the active tab.
  const jobs = [fetchAndRenderSummary()];

  if (activeTab === "tasks") {
    jobs.push(fetchAndRenderTasks());
    if (!document.hidden) jobs.push(fetchAndRenderTaskLocks());
    return jobs;
  }

  if (activeTab === "scoreboard") {
    jobs.push(fetchJson("/api/scoreboard").then(renderScoreboard));
    return jobs;
  }

  if (activeTab === "audit") {
    if (getActiveAuditSubtab() === "policy") {
      jobs.push(fetchAndRenderPolicy(auditContext()));
    } else {
      jobs.push(fetchAndRenderAudit(auditContext()));
    }
    return jobs;
  }

  if (activeTab === "knowledge") {
    if (activeKnowledgeSubtab === "adrs") {
      jobs.push(fetchAndRenderAdrs());
    } else if (activeKnowledgeSubtab === "frictions") {
      jobs.push(fetchAndRenderFrictions());
    } else {
      jobs.push(fetchAndRenderLearnings());
    }
    return jobs;
  }

  if (activeTab === "run-detail") {
    if (!activeRunId) {
      renderRunDetailEmpty("No run selected.");
      return jobs;
    }
    jobs.push(fetchAndRenderRunDetail());
    // Events power both the Events sub-tab and the Gantt's retry markers, so
    // they're fetched on every run-detail refresh regardless of which sub-tab
    // is active.
    jobs.push(fetchAndRenderRunEvents());
    jobs.push(fetchAndRenderRunLogs());
    return jobs;
  }

  if (activeTab === "diagnostics") {
    if (activeDiagSubtab === "runs") {
      jobs.push(fetchAndRenderRuns());
    } else if (activeDiagSubtab === "metrics") {
      jobs.push(
        fetchJson(`/api/diagnostics/metrics?limit=${DIAG_LIMIT}`).then((rows) => {
          lastDiagnostics.metrics = rows;
          renderDiagnostics(diagnosticsContext());
        })
      );
    } else if (activeDiagSubtab === "errors") {
      jobs.push(
        fetchJson(`/api/diagnostics/errors?limit=${DIAG_LIMIT}`).then((rows) => {
          lastDiagnostics.errors = rows;
          renderDiagnostics(diagnosticsContext());
        })
      );
    }

    jobs.push(
      fetchJson(`/api/diagnostics/implement_one`)
        .then((implOne) => {
          lastDiagnostics.implement_one = implOne.implement_one_by_actor || [];
          const sidePanel = $("diagnostics-side-panel");
          if (sidePanel) {
            renderImplOne($("diag-implement-one-body"), lastDiagnostics.implement_one, diagnosticsContext());
          }
        })
        .catch(e => console.error("Failed to fetch implement_one metrics", e))
    );
  }
  return jobs;
}

function fetchAndRenderRuns() {
  return Promise.all([
    fetchJson(`/api/job-runs?limit=${JOB_RUN_LIMIT}`),
    fetchJson(`/api/diagnostics/friction?limit=${DIAG_LIMIT}`),
  ]).then(([runs, frictionRows]) => {
    lastRuns = mergeRunsWithFriction(runs, frictionRows);
    renderRuns(lastRuns);
  });
}

function fetchAndRenderTaskLocks() {
  return fetchJson("/api/tasks/locks").then(renderLocksPanel);
}

function fetchAndRenderRunDetail() {
  if (!activeRunId) return Promise.resolve();
  return fetchJson(`/api/runs/${encodeURIComponent(activeRunId)}`).then((data) => {
    activeRunDetail = data;
    renderRunDetailMeta();
    renderRunKnowledge();
    renderRunGantt();
    renderRunSteps();
  }).catch((e) => {
    renderRunDetailEmpty(`Run not found: ${activeRunId}`);
    throw e;
  });
}

function fetchAndRenderRunEvents() {
  if (!activeRunId) return Promise.resolve();
  return fetchJson(`/api/runs/${encodeURIComponent(activeRunId)}/events?limit=${RUN_EVENTS_LIMIT}`).then((events) => {
    activeRunEvents = events;
    renderRunEvents();
    renderRunGantt();
  }).catch(() => {
    // Missing v2 events file is non-fatal — run detail still renders.
    activeRunEvents = [];
    renderRunGantt();
  });
}

function fetchAndRenderRunLogs() {
  if (!activeRunId) return Promise.resolve();
  return fetchJson(`/api/runs/${encodeURIComponent(activeRunId)}/logs?limit=${RUN_EVENTS_LIMIT}`).then((logs) => {
    activeRunLogs = logs;
    renderRunSteps();
  }).catch(() => {
    activeRunLogs = [];
    renderRunSteps();
  });
}

function fetchAndRenderSummary() {
  return fetchJson(`/api/audit/summary?since=24h`).then((data) => {
    lastSummary = data;
    renderHealthStrip(data);
    renderAuditSummary(data, auditContext());
  });
}

function fetchAndRenderLearnings() {
  const sp = new URLSearchParams();
  sp.set("limit", String(LEARNING_LIMIT));
  if (learningSearchQuery) sp.set("q", learningSearchQuery);
  return fetchJson(`/api/learnings?${sp.toString()}`).then((payload) => {
    lastLearningPayload = payload || { stats: {}, items: [] };
    renderLearnings(lastLearningPayload);
  });
}

function fetchAndRenderAdrs() {
  const sp = new URLSearchParams();
  sp.set("limit", String(ADR_LIMIT));
  if (adrSearchQuery) sp.set("q", adrSearchQuery);
  return fetchJson(`/api/adrs?${sp.toString()}`).then((payload) => {
    lastAdrPayload = payload || { stats: {}, items: [] };
    renderAdrs(lastAdrPayload);
  });
}

function fetchAndRenderFrictions() {
  const sp = new URLSearchParams();
  sp.set("limit", String(FRICTION_LIMIT));
  if (frictionSearchQuery) sp.set("q", frictionSearchQuery);
  return Promise.all([
    fetchJson(`/api/frictions?${sp.toString()}`),
    fetchJson("/api/frictions/stats"),
  ]).then(([payload, stats]) => {
    lastFrictionPayload = payload || { stats: {}, tags: [], items: [] };
    lastFrictionPayload.stats = stats || lastFrictionPayload.stats || {};
    renderFrictions(lastFrictionPayload);
  });
}

function renderHealthStrip(data) {
  if (!data) return;
  $("tile-events-value").textContent = formatBigInt(data.events);
  $("tile-denials-value").textContent = formatBigInt(data.denials);
  $("tile-failed-value").textContent = formatBigInt(data.failed_runs);
  $("tile-active-value").textContent = formatBigInt(data.active_long_runs);
  const tile = $("tile-denials");
  const threshold = data.denial_threshold ?? 10;
  if (data.denials > threshold) {
    tile.classList.add("tile-alert");
  } else {
    tile.classList.remove("tile-alert");
  }
  renderSparkline(data.sparkline || []);
}

function formatBigInt(n) {
  if (n == null) return "-";
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function renderSparkline(buckets) {
  const svg = $("tile-events-sparkline");
  if (!svg) return;
  while (svg.firstChild) svg.removeChild(svg.firstChild);
  if (buckets.length === 0) return;
  const counts = buckets.map((b) => b.count || 0);
  const max = Math.max(1, ...counts);
  const w = 100;
  const h = 22;
  const stepX = buckets.length > 1 ? w / (buckets.length - 1) : 0;
  const points = counts.map((c, i) => {
    const x = i * stepX;
    const y = h - (c / max) * (h - 2) - 1;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  const baseline = document.createElementNS("http://www.w3.org/2000/svg", "line");
  baseline.setAttribute("x1", "0");
  baseline.setAttribute("y1", String(h - 0.5));
  baseline.setAttribute("x2", String(w));
  baseline.setAttribute("y2", String(h - 0.5));
  baseline.setAttribute("class", "baseline");
  svg.appendChild(baseline);
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", `M${points.join(" L")}`);
  svg.appendChild(path);
}

function refreshLabel() {
  if (activeTab === "diagnostics") return `diagnostics/${activeDiagSubtab}`;
  if (activeTab === "run-detail") return `run/${activeRunId || "?"}`;
  return activeTab;
}

function navigateToRun(runId) {
  activeRunId = runId;
  expandedStepIndices = new Set();
  activeRunDetail = null;
  activeRunEvents = [];
  setActiveTab(`runs/${encodeURIComponent(runId)}`);
}



function renderRunDetailEmpty(message) {
  const meta = $("run-detail-meta");
  if (meta) syncNodes(meta, [el("div", { class: "empty-state" }, [
    el("div", { class: "icon", text: "✧" }),
    el("div", { class: "text", text: message }),
  ])]);
  $("run-detail-title").textContent = "Run Detail";
  $("run-detail-count").textContent = "-";
  $("run-steps-body").innerHTML = "";
  $("run-events-body").innerHTML = "";
  const knowledge = $("run-knowledge-panel");
  const gantt = $("run-gantt-panel");
  if (knowledge) knowledge.style.display = "none";
  if (gantt) gantt.style.display = "none";
}

function renderRunDetailMeta() {
  const meta = $("run-detail-meta");
  if (!meta) return;
  const detail = activeRunDetail || {};
  const run = detail.run || {};
  $("run-detail-title").textContent = `Run ${run.run_id || activeRunId || "?"}`;
  const stepCount = Array.isArray(detail.steps) ? detail.steps.length : 0;
  $("run-detail-count").textContent = `${stepCount} steps`;

  const grid = el("div", { class: "run-meta-grid" });
  const addCell = (label, value) => {
    const cell = el("div");
    cell.appendChild(el("div", { class: "label", text: label }));
    cell.appendChild(el("div", { class: "value", text: value == null ? "-" : String(value) }));
    grid.appendChild(cell);
  };
  addCell("job", run.job_id);
  addCell("state", run.state);
  addCell("attempt", run.attempt);
  addCell("started", run.started_at ? fmtAbsTime(run.started_at) : "-");
  addCell("finished", run.finished_at ? fmtAbsTime(run.finished_at) : "-");
  addCell("duration", run.duration_ms != null ? fmtDuration(run.duration_ms) : "-");

  const wrap = el("div");
  const back = el("button", { class: "back-action", text: "← back to runs" });
  back.addEventListener("click", () => setActiveTab("diagnostics/runs"));
  const actions = el("div", { class: "run-detail-actions" }, [back]);
  if (run.retry_source_run_id) {
    const sourceId = run.retry_source_run_id;
    const lineage = el("button", {
      class: "back-action replay-source",
      text: `Replayed from ${sourceId}`,
      title: `Open ${sourceId}`,
    });
    lineage.addEventListener("click", () => navigateToRun(sourceId));
    actions.appendChild(lineage);
  }
  if (run.run_id) actions.appendChild(buildReplayRunButton(run, wrap));
  if (runIsCancellable(run)) actions.appendChild(buildCancelRunButton(run, wrap));
  wrap.appendChild(actions);
  wrap.appendChild(grid);
  syncNodes(meta, [wrap]);
}

function renderRunSteps() {
  const body = $("run-steps-body");
  if (!body) return;
  const steps = (activeRunDetail && activeRunDetail.steps) || [];
  if (steps.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No steps recorded for this run." }),
    ])]);
    return;
  }
  const frag = document.createDocumentFragment();
  for (const step of steps) {
    const exit = step.exit_code;
    const exitClass = exit != null && exit !== 0 ? "exit fail" : "exit";
    const row = el("div", { class: "step-row" }, [
      el("span", { class: "idx", text: `#${step.step_index}` }),
      el("span", { class: "target", text: `${step.target_type}:${step.target_id}` }),
      el("span", {}, [stateCell(step.state)]),
      el("span", { class: "duration", text: fmtDuration(step.duration_ms) }),
      el("span", { class: exitClass, text: exit == null ? "-" : String(exit) }),
    ]);
    row.dataset.key = `step-${step.step_index}`;
    row.dataset.hash = `${step.step_index}-${step.state}-${exit}`;
    if (expandedStepIndices.has(step.step_index)) row.classList.add("expanded");
    row.addEventListener("click", () => {
      if (expandedStepIndices.has(step.step_index)) expandedStepIndices.delete(step.step_index);
      else expandedStepIndices.add(step.step_index);
      renderRunSteps();
    });
    frag.appendChild(row);
    if (expandedStepIndices.has(step.step_index)) {
      frag.appendChild(buildStepDetail(step));
    }
  }
  syncNodes(body, Array.from(frag.children));
}

function renderRunKnowledge() {
  const panel = $("run-knowledge-panel");
  if (!panel) return;
  panel.style.display = "block";
  const km = activeRunDetail && activeRunDetail.run && activeRunDetail.run.knowledge_metrics;
  panel.innerHTML = "";
  const header = el("div", { class: "knowledge-header", text: "Knowledge Pack" });
  panel.appendChild(header);
  if (km == null) {
    panel.appendChild(el("div", { class: "knowledge-empty", text: "no knowledge metrics for this run" }));
    return;
  }
  const grid = el("div", { class: "knowledge-grid" });
  const baseline = Number(km.raw_read_token_baseline || 0);
  const packTokens = km.knowledge_pack_tokens == null ? null : Number(km.knowledge_pack_tokens);
  const totalLlm = km.total_llm_input_tokens == null ? null : Number(km.total_llm_input_tokens);
  const ratioText = baseline === 0 || packTokens == null
    ? "n/a"
    : `${((packTokens / baseline) * 100).toFixed(1)}%`;
  const addCell = (label, value, extra = "") => {
    const cell = el("div");
    cell.appendChild(el("div", { class: "label", text: label }));
    cell.appendChild(el("div", { class: `value${extra ? " " + extra : ""}`, text: value }));
    grid.appendChild(cell);
  };
  addCell("raw_read_token_baseline", String(baseline));
  addCell("knowledge_pack_tokens", packTokens == null ? "-" : String(packTokens));
  addCell("total_llm_input_tokens", totalLlm == null ? "-" : String(totalLlm));
  addCell("compression_ratio", ratioText, "ratio");
  panel.appendChild(grid);
}

function renderRunGantt() {
  const panel = $("run-gantt-panel");
  if (!panel) return;
  const detail = activeRunDetail || {};
  const run = detail.run || {};
  const steps = Array.isArray(detail.steps) ? detail.steps : [];
  if (steps.length === 0) {
    panel.style.display = "none";
    panel.innerHTML = "";
    return;
  }
  panel.style.display = "block";
  panel.innerHTML = "";
  panel.appendChild(el("div", { class: "gantt-header", text: "Step Timeline" }));

  const startMs = run.started_at ? new Date(run.started_at).getTime() : null;
  let endMs = run.finished_at ? new Date(run.finished_at).getTime() : null;
  // Walk steps for tighter bounds when run-level timestamps are missing.
  let derivedStart = startMs;
  let derivedEnd = endMs;
  for (const s of steps) {
    if (s.started_at) {
      const t = new Date(s.started_at).getTime();
      if (derivedStart == null || t < derivedStart) derivedStart = t;
    }
    if (s.finished_at) {
      const t = new Date(s.finished_at).getTime();
      if (derivedEnd == null || t > derivedEnd) derivedEnd = t;
    }
  }
  // Run still in flight or missing finish: extend to now.
  if (derivedEnd == null) derivedEnd = Date.now();
  if (derivedStart == null) derivedStart = derivedEnd - 1000;
  if (derivedEnd <= derivedStart) derivedEnd = derivedStart + 1000;

  const PAD_LEFT = 56;   // step-index gutter
  const PAD_RIGHT = 12;
  const PAD_TOP = 18;
  const ROW_H = 22;
  const BAR_H = 14;
  const W_TOTAL = 1000;  // virtual viewBox width; SVG rescales to container
  const innerW = W_TOTAL - PAD_LEFT - PAD_RIGHT;
  const totalH = PAD_TOP + steps.length * ROW_H + 18;
  const span = derivedEnd - derivedStart;
  const xOf = (ms) => PAD_LEFT + ((ms - derivedStart) / span) * innerW;

  const svgNS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("class", "gantt-svg");
  svg.setAttribute("viewBox", `0 0 ${W_TOTAL} ${totalH}`);
  svg.setAttribute("preserveAspectRatio", "none");
  svg.style.height = `${totalH}px`;

  // Lane backgrounds + step-index labels.
  steps.forEach((step, i) => {
    const y = PAD_TOP + i * ROW_H;
    const bg = document.createElementNS(svgNS, "rect");
    bg.setAttribute("class", `gantt-lane-bg${i % 2 === 0 ? "" : " alt"}`);
    bg.setAttribute("x", String(0));
    bg.setAttribute("y", String(y));
    bg.setAttribute("width", String(W_TOTAL));
    bg.setAttribute("height", String(ROW_H));
    svg.appendChild(bg);
    const label = document.createElementNS(svgNS, "text");
    label.setAttribute("class", "gantt-lane-label");
    label.setAttribute("x", String(8));
    label.setAttribute("y", String(y + ROW_H / 2 + 3));
    label.textContent = `#${step.step_index}`;
    svg.appendChild(label);
  });

  // Axis: start and end labels.
  const axisY = PAD_TOP + steps.length * ROW_H + 6;
  const axisLine = document.createElementNS(svgNS, "line");
  axisLine.setAttribute("class", "gantt-axis-line");
  axisLine.setAttribute("x1", String(PAD_LEFT));
  axisLine.setAttribute("y1", String(axisY));
  axisLine.setAttribute("x2", String(W_TOTAL - PAD_RIGHT));
  axisLine.setAttribute("y2", String(axisY));
  svg.appendChild(axisLine);
  const fmtAxis = (ms) => {
    const d = new Date(ms);
    const pad = (n) => String(n).padStart(2, "0");
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  };
  const axisStart = document.createElementNS(svgNS, "text");
  axisStart.setAttribute("class", "gantt-axis-label");
  axisStart.setAttribute("x", String(PAD_LEFT));
  axisStart.setAttribute("y", String(axisY + 12));
  axisStart.textContent = fmtAxis(derivedStart);
  svg.appendChild(axisStart);
  const axisEnd = document.createElementNS(svgNS, "text");
  axisEnd.setAttribute("class", "gantt-axis-label");
  axisEnd.setAttribute("x", String(W_TOTAL - PAD_RIGHT));
  axisEnd.setAttribute("y", String(axisY + 12));
  axisEnd.setAttribute("text-anchor", "end");
  axisEnd.textContent = fmtAxis(derivedEnd);
  svg.appendChild(axisEnd);

  // Bars per step.
  steps.forEach((step, i) => {
    const sStart = step.started_at ? new Date(step.started_at).getTime() : derivedStart;
    const sEnd = step.finished_at ? new Date(step.finished_at).getTime() : derivedEnd;
    const x1 = xOf(sStart);
    const x2 = xOf(sEnd);
    const w = Math.max(2, x2 - x1);
    const y = PAD_TOP + i * ROW_H + (ROW_H - BAR_H) / 2;
    const bar = document.createElementNS(svgNS, "rect");
    bar.setAttribute("class", "gantt-bar");
    bar.setAttribute("x", String(x1));
    bar.setAttribute("y", String(y));
    bar.setAttribute("width", String(w));
    bar.setAttribute("height", String(BAR_H));
    bar.setAttribute("fill", `var(--state-${step.state}, var(--fg-dim))`);
    bar.addEventListener("mousemove", (e) => showGanttTooltip(e, step));
    bar.addEventListener("mouseleave", hideGanttTooltip);
    bar.addEventListener("click", () => {
      // Reuse the per-step expand/collapse panel from the Steps sub-tab.
      if (expandedStepIndices.has(step.step_index)) {
        expandedStepIndices.delete(step.step_index);
      } else {
        expandedStepIndices.add(step.step_index);
      }
      activeRunSubtab = "steps";
      setRunDetailSubtab("steps");
      renderRunSteps();
      const target = document.querySelector(`[data-key="step-${step.step_index}"]`);
      if (target && target.scrollIntoView) {
        target.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
    });
    svg.appendChild(bar);
  });

  // Retry markers from StepRetry events.
  const stepIdToIndex = new Map();
  steps.forEach((s) => {
    // step_id is the activity name (or step.id); the step file stores it as
    // target_id. Map both forms to the lane index.
    if (s.target_id != null) stepIdToIndex.set(String(s.target_id), s.step_index);
  });
  for (const ev of activeRunEvents || []) {
    if (ev.body_kind !== "step_retry") continue;
    const stepId = ev.step_id;
    const index = stepIdToIndex.get(stepId);
    if (index == null) continue;
    const tsMs = ev.ts ? new Date(ev.ts).getTime() : null;
    if (!tsMs || isNaN(tsMs)) continue;
    const cx = xOf(Math.max(derivedStart, Math.min(derivedEnd, tsMs)));
    const cy = PAD_TOP + index * ROW_H + ROW_H / 2;
    const marker = document.createElementNS(svgNS, "circle");
    marker.setAttribute("class", "gantt-retry-marker");
    marker.setAttribute("cx", String(cx));
    marker.setAttribute("cy", String(cy));
    marker.setAttribute("r", "3.5");
    const title = document.createElementNS(svgNS, "title");
    title.textContent = `retry attempt=${ev.attempt} backoff=${ev.next_backoff_ms}ms`;
    marker.appendChild(title);
    svg.appendChild(marker);
  }

  panel.appendChild(svg);
}

function showGanttTooltip(e, step) {
  const tip = $("gantt-tooltip");
  if (!tip) return;
  const lines = [
    `step_index: ${step.step_index}`,
    `state: ${step.state}`,
    `exit_code: ${step.exit_code == null ? "-" : step.exit_code}`,
    `duration_ms: ${step.duration_ms == null ? "-" : step.duration_ms}`,
  ];
  tip.textContent = lines.join("\n");
  tip.style.display = "block";
  tip.setAttribute("aria-hidden", "false");
  // Position relative to viewport; clamp to keep the tooltip on-screen.
  const x = Math.min(window.innerWidth - 220, e.clientX + 12);
  const y = Math.min(window.innerHeight - 80, e.clientY + 12);
  tip.style.left = `${x}px`;
  tip.style.top = `${y}px`;
}

function hideGanttTooltip() {
  const tip = $("gantt-tooltip");
  if (!tip) return;
  tip.style.display = "none";
  tip.setAttribute("aria-hidden", "true");
}

function logsForStep(step) {
  const stepId = step.target_id == null ? null : String(step.target_id);
  return (activeRunLogs || []).filter((record) => {
    if (record.step_index != null && Number(record.step_index) === Number(step.step_index)) return true;
    return stepId != null && record.step_id === stepId;
  });
}

function buildLogBlock(record, stream) {
  const isErr = stream === "stderr";
  const preview = record[`${stream}_preview`] || "";
  if (!preview) return null;
  const block = el("div", { class: `step-log-block ${isErr ? "stderr" : "stdout"}` });
  const meta = [
    record.provider || "cli",
    record.exit_code == null ? "exit -" : `exit ${record.exit_code}`,
    record.timed_out ? "timeout" : null,
    record[`${stream}_truncated`] ? "truncated" : null,
  ].filter(Boolean).join(" · ");
  block.appendChild(el("div", { class: "step-log-head" }, [
    el("span", { class: "label", text: stream }),
    el("span", { class: "meta", text: meta }),
  ]));
  const pre = el("pre");
  for (const line of preview.split("\n")) {
    const row = el("span", {
      class: isErr && /\bERROR\s+[^:]+:/.test(line) ? "log-line error-line" : "log-line",
      text: line || " ",
    });
    pre.appendChild(row);
  }
  block.appendChild(pre);
  return block;
}

function buildStepDetail(step) {
  const wrap = el("div", { class: "step-detail" });
  wrap.dataset.key = `step-detail-${step.step_index}`;
  wrap.dataset.hash = JSON.stringify(step);
  wrap.addEventListener("click", (e) => e.stopPropagation());

  const addBlock = (label, raw) => {
    const v = raw == null ? "" : (typeof raw === "string" ? raw : JSON.stringify(raw, null, 2));
    if (!v) return;
    const block = el("div", { class: "audit-detail-block" });
    block.appendChild(el("div", { class: "label", text: label }));
    block.appendChild(el("pre", { text: v }));
    wrap.appendChild(block);
  };

  if (step.error_message) addBlock("error", `${step.error_code || ""} ${step.error_message}`);
  addBlock("agent_response", step.agent_response_json);
  const logs = logsForStep(step);
  if (logs.length > 0) {
    const section = el("div", { class: "step-log-section" });
    section.appendChild(el("div", { class: "label", text: "agent logs" }));
    for (const record of logs) {
      const stdout = buildLogBlock(record, "stdout");
      const stderr = buildLogBlock(record, "stderr");
      if (stdout) section.appendChild(stdout);
      if (stderr) section.appendChild(stderr);
    }
    wrap.appendChild(section);
  }
  const km = activeRunDetail && activeRunDetail.run && activeRunDetail.run.knowledge_metrics;
  if (km && step.step_index === 0) addBlock("knowledge_metrics (run)", km);
  return wrap;
}

const RUN_EVENT_COLUMNS = [
  { key: "ts", label: "time" },
  { key: "body_kind", label: "kind" },
  { key: "event_type", label: "scope" },
  { key: "agent_identity", label: "agent" },
  { key: "summary", label: "detail" },
];

function renderRunEvents() {
  const body = $("run-events-body");
  if (!body) return;
  const events = activeRunEvents || [];
  if (events.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No v2 envelope events for this run." }),
    ])]);
    return;
  }
  let table = body.querySelector("table.scoreboard-table");
  let tbody;
  if (!table) {
    table = el("table", { class: "scoreboard-table" });
    const thead = el("thead");
    const headRow = el("tr");
    for (const col of RUN_EVENT_COLUMNS) {
      headRow.appendChild(el("th", { text: col.label }));
    }
    thead.appendChild(headRow);
    table.appendChild(thead);
    tbody = el("tbody");
    table.appendChild(tbody);
    syncNodes(body, [table]);
  } else {
    tbody = table.querySelector("tbody");
  }
  const frag = document.createDocumentFragment();
  for (let i = 0; i < events.length; i++) {
    const ev = events[i];
    const summary = summarizeEvent(ev);
    const tr = el("tr");
    tr.appendChild(el("td", { text: fmtTimestamp(ev.ts) }));
    tr.appendChild(el("td", { text: ev.body_kind || "-" }));
    tr.appendChild(el("td", { text: ev.event_type || "-" }));
    tr.appendChild(el("td", { text: ev.agent_identity || "-" }));
    const td = el("td", { class: "stderr" });
    td.title = summary.title;
    td.textContent = summary.text;
    tr.appendChild(td);
    tr.dataset.key = `runev-${ev.event_id || i}`;
    tr.dataset.hash = `${ev.event_id || i}-${ev.body_kind}`;
    frag.appendChild(tr);
  }
  syncNodes(tbody, Array.from(frag.children));
}

function summarizeEvent(ev) {
  const ignoreKeys = new Set([
    "schemaVersion", "event_type", "event_id", "ts", "run_id",
    "agent_identity", "parent_event_id", "workspace_path", "body_kind",
  ]);
  const parts = [];
  for (const [k, v] of Object.entries(ev)) {
    if (ignoreKeys.has(k)) continue;
    if (v == null) continue;
    if (typeof v === "object") {
      parts.push(`${k}=${JSON.stringify(v)}`);
    } else {
      parts.push(`${k}=${v}`);
    }
  }
  const text = parts.join(" ");
  return { text: truncate(text, 200), title: text };
}

async function refreshDashboard() {
  if (isRefreshing) return;
  isRefreshing = true;
  const now = new Date();
  $("meta-text").textContent = `fetching...`;
  $("conn-status").className = "status-dot orange";
  const btn = $("refresh-btn");
  if (btn) btn.disabled = true;
  
  let hasErrors = false;
  
  await Promise.all(
    activeRefreshJobs().map((job) =>
      job.catch((e) => {
        hasErrors = true;
        console.error(e);
      }),
    ),
  );
  
  if (hasErrors) {
    $("conn-status").className = "status-dot red";
    $("meta-text").textContent = `offline · ${now.toLocaleTimeString()}`;
  } else {
    $("conn-status").className = "status-dot green";
    $("meta-text").textContent = `refreshed ${refreshLabel()} · ${now.toLocaleTimeString()}`;
  }
  if (btn) btn.disabled = false;
  isRefreshing = false;
  if (activeTab === "tasks") fitLogPanelToViewport();
}

const tasksContext = taskContext();
buildChips(tasksContext);
wireSearch(tasksContext);
wireLearningSearch();
wireAdrSearch();
wireFrictionSearch();
buildAuditChips(auditContext());
wireAuditSearch(auditContext());
$("refresh-btn").addEventListener("click", refreshDashboard);
initTabs();

initLogTail();

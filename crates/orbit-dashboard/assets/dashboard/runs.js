// Orbit dashboard runs-domain (Recent Runs table, cancel/replay, friction badges, sort).
// Pure vanilla JS, split into ES module with no build step.
//
// Extracted from app.js (ORB-00180). Owns the runSort state, all friction-row
// classification helpers (coerce*, first*, frictionRow*, empty*, add*), merge*,
// sort*, header/cell renderers, and the table renderer.
// Also owns the action button builders and cancel/replay fns (used by both the
// runs table and the run-detail meta in app.js).
//
// Receives one-time context via initRuns(runsContext()) containing the
// callbacks (fetchAndRender*, navigateToRun) and getters (activeRunId, lastRuns,
// formatters) that the actions and render depend on. No direct import from app.js.

import { el, stateCell, syncNodes, postJson } from './common.js';

const $ = (id) => document.getElementById(id);

const CANCELLABLE_RUN_STATES = new Set(["pending", "running"]);

const RUN_SORT_DEFAULT_DIR = {
  when: "desc",
  job: "asc",
  run_id: "asc",
  denials: "desc",
  tool_fails: "desc",
  duration: "desc",
  state: "asc",
};

let runSort = { key: "when", dir: "desc" };

let _runsCtx = null;

function hasCtx(key) {
  const c = _runsCtx;
  return !!(c && typeof c[key] === "function");
}

function getActiveRunId() {
  return hasCtx("getActiveRunId") ? _runsCtx.getActiveRunId() : null;
}

function doNavigateToRun(runId) {
  if (hasCtx("navigateToRun")) {
    try { _runsCtx.navigateToRun(runId); } catch (_) {}
  }
}

function doFetchAndRenderRuns() {
  if (hasCtx("fetchAndRenderRuns")) {
    try { return _runsCtx.fetchAndRenderRuns(); } catch (_) { return Promise.resolve(); }
  }
  return Promise.resolve();
}

function doFetchAndRenderRunDetail() {
  if (hasCtx("fetchAndRenderRunDetail")) {
    try { return _runsCtx.fetchAndRenderRunDetail(); } catch (_) { return Promise.resolve(); }
  }
  return Promise.resolve();
}

function doFetchAndRenderRunEvents() {
  if (hasCtx("fetchAndRenderRunEvents")) {
    try { return _runsCtx.fetchAndRenderRunEvents(); } catch (_) { return Promise.resolve(); }
  }
  return Promise.resolve();
}

function fmtTimestampValue(v) {
  return hasCtx("fmtTimestamp") ? _runsCtx.fmtTimestamp(v) : (v || "-");
}

function fmtDurationValue(v) {
  return hasCtx("fmtDuration") ? _runsCtx.fmtDuration(v) : (v == null ? "-" : String(v));
}

export function initRuns(ctx) {
  _runsCtx = ctx;
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
      doFetchAndRenderRuns(),
      getActiveRunId() === runId ? doFetchAndRenderRunDetail() : Promise.resolve(),
      getActiveRunId() === runId ? doFetchAndRenderRunEvents() : Promise.resolve(),
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
    doNavigateToRun(payload.run_id);
    doFetchAndRenderRuns().catch(console.error);
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

export function mergeRunsWithFriction(runs, frictionRows) {
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
  const data = hasCtx("getLastRuns") ? _runsCtx.getLastRuns() : [];
  renderRuns(data);
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
    fmtDurationValue(friction.durationMs),
    friction.longRun
      ? el("span", { class: "long-run-flag", text: "!", title: "Long run" })
      : null,
  ]);
}

export function renderRuns(runs) {
  const body = $("runs-body");
  const frag = document.createDocumentFragment();
  
  const sorted = sortedRunsForDisplay(runs);
  const top = sorted.slice(0, 20);
  if ($("diag-count")) {
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
      el("span", { class: "when", text: fmtTimestampValue(ts) }),
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
    row.addEventListener("click", () => doNavigateToRun(r.run_id));
    frag.appendChild(row);
  }
  syncNodes(body, Array.from(frag.children));
}

export {
  runIsCancellable,
  buildCancelRunButton,
  buildReplayRunButton,
};

// Orbit dashboard router (hash routing + top-tab + subtab wiring).
// Pure vanilla JS, split into ES module with no build step.
//
// Moved from app.js (router was the widest cross-cut for subsequent module splits).
// Owns: TABS, DIAG_SUBTABS, METRICS_SUBTABS, RUN_DETAIL_SUBTABS, KNOWLEDGE_SUBTABS, parseHashRoute,
// setActiveTab, set*Subtab helpers, initTabs, navigateToRun.
//
// All mutable dashboard state remains in app.js. Router receives a context object
// (from app.js routerContext()) providing getters/setters + callbacks for renders,
// refresh, log fit, and audit pass-throughs. This avoids circular imports and keeps
// the "state lives in app" contract.
//
// The public exports (setActiveTab, navigateToRun, initTabs) are thin wrappers over
// impls that close over the injected ctx (set once via initRouter).
// No behavior change: every prior hash route, subtab click, back/forward, and the
// 30s setInterval(refreshDashboard) continue to work identically.
//
// Also exports parseHashRoute for symmetry (used only internally today).

import { el } from './common.js';
import { renderRuns } from './runs.js';

const $ = (id) => document.getElementById(id);

const TABS = ["tasks", "scoreboard", "metrics", "audit", "diagnostics", "knowledge", "run-detail"];
const DIAG_SUBTABS = ["runs", "metrics", "errors"];
const METRICS_SUBTABS = ["knowledge", "activity", "tools", "task", "invocations"];
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

let _routerCtx = null;

function getCtx() {
  if (!_routerCtx) {
    throw new Error("router not initialized; call initRouter(routerContext()) before using router APIs");
  }
  return _routerCtx;
}

function setRunDetailSubtabImpl(ctx, name) {
  if (!RUN_DETAIL_SUBTABS.includes(name)) name = "steps";
  ctx.setRunSubtab(name);
  for (const btn of document.querySelectorAll("#run-detail-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }
  $("run-steps-body").style.display = name === "steps" ? "block" : "none";
  $("run-events-body").style.display = name === "events" ? "block" : "none";
}

function setDiagSubtabImpl(ctx, name) {
  if (!DIAG_SUBTABS.includes(name)) name = "runs";
  ctx.setDiagSubtab(name);
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
    renderRuns(ctx.getLastRuns ? ctx.getLastRuns() : []);
  } else {
    $("diag-body").style.display = "block";
    $("runs-body").style.display = "none";
    ctx.renderDiagnostics();
  }
}

function setMetricsSubtabImpl(ctx, name) {
  if (!METRICS_SUBTABS.includes(name)) name = "knowledge";
  ctx.setMetricsSubtab(name);
  for (const btn of document.querySelectorAll("#metrics-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }
  if (typeof ctx.renderMetrics === "function") ctx.renderMetrics();
}

function setKnowledgeSubtabImpl(ctx, name) {
  if (!KNOWLEDGE_SUBTABS.includes(name)) name = "learnings";
  ctx.setKnowledgeSubtab(name);
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

function setActiveTabImpl(ctx, raw, opts = {}) {
  const { segments, query } = parseHashRoute(raw);
  const head = segments[0] || "tasks";
  if (head === "runs" && !segments[1] && query.get("run_id")) {
    segments[1] = encodeURIComponent(query.get("run_id"));
  }
  let top;
  if (head === "runs" && segments[1]) {
    top = "run-detail";
    const nextRunId = decodeURIComponent(segments[1]);
    if (ctx.getRunId() !== nextRunId) {
      ctx.setRunLogs([]);
      const esi = ctx.getExpandedSteps();
      if (esi && typeof esi.clear === "function") esi.clear();
      else ctx.setExpandedSteps(new Set());
    }
    ctx.setRunId(nextRunId);
    const expandStep = query.get("step");
    if (expandStep != null && /^\d+$/.test(expandStep)) {
      const esi = ctx.getExpandedSteps();
      if (esi && typeof esi.add === "function") esi.add(Number(expandStep));
      else {
        const s = new Set(esi || []);
        s.add(Number(expandStep));
        ctx.setExpandedSteps(s);
      }
    }
    const sub = RUN_DETAIL_SUBTABS.includes(segments[2]) ? segments[2] : ctx.getRunSubtab();
    ctx.setRunSubtab(sub);
  } else if (TABS.includes(head)) {
    top = head;
  } else {
    top = "tasks";
  }
  ctx.setTab(top);
  for (const tab of document.querySelectorAll(".tab")) {
    tab.classList.toggle("active", tab.dataset.tab === top);
  }
  for (const pane of document.querySelectorAll(".tab-pane")) {
    pane.classList.toggle("active", pane.dataset.tab === top);
  }
  if (top === "tasks") requestAnimationFrame(ctx.fitLogPanelToViewport || (() => {}));

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
    const sub = DIAG_SUBTABS.includes(segments[1]) ? segments[1] : ctx.getDiagSubtab();
    setDiagSubtabImpl(ctx, sub);
    hash = `#diagnostics/${sub}`;
  } else if (top === "metrics") {
    const sub = METRICS_SUBTABS.includes(segments[1]) ? segments[1] : ctx.getMetricsSubtab();
    setMetricsSubtabImpl(ctx, sub);
    hash = `#metrics/${sub}`;
  } else if (top === "audit") {
    ctx.applyAuditHashQuery(query);
    const sub = ["events", "policy"].includes(segments[1]) ? segments[1] : ctx.getActiveAuditSubtab();
    ctx.setAuditSubtab(sub);
    hash = ctx.buildAuditHash();
    ctx.syncAuditControls();
  } else if (top === "run-detail") {
    setRunDetailSubtabImpl(ctx, ctx.getRunSubtab());
    hash = `#runs/${encodeURIComponent(ctx.getRunId() || "")}` +
      (ctx.getRunSubtab() !== "steps" ? `/${ctx.getRunSubtab()}` : "");
    if (query.get("step") != null) hash += `?step=${encodeURIComponent(query.get("step"))}`;
  } else if (top === "knowledge") {
    const sub = KNOWLEDGE_SUBTABS.includes(segments[1]) ? segments[1] : ctx.getKnowledgeSubtab();
    setKnowledgeSubtabImpl(ctx, sub);
    hash = sub === "learnings" ? "#knowledge/learnings" : `#knowledge/${sub}`;
  } else {
    hash = `#${top}`;
  }
  const hashChanged = window.location.hash !== hash;
  const shouldUpdateHash = opts.updateHash !== false;
  if (hashChanged && shouldUpdateHash) {
    window.location.hash = hash;
  }
  if (opts.refresh !== false && (!hashChanged || !shouldUpdateHash)) ctx.refreshDashboard();
}

function navigateToRunImpl(ctx, runId) {
  ctx.setRunId(runId);
  ctx.setExpandedSteps(new Set());
  ctx.setRunDetail(null);
  ctx.setRunEvents([]);
  setActiveTabImpl(ctx, `runs/${encodeURIComponent(runId)}`);
}

function initTabsImpl(ctx) {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.addEventListener("click", () => setActiveTabImpl(ctx, tab.dataset.tab, { refresh: false }));
  }
  for (const btn of document.querySelectorAll("#diag-subtabs .subtab")) {
    btn.addEventListener("click", () =>
      setActiveTabImpl(ctx, `diagnostics/${btn.dataset.subtab}`, { refresh: false }),
    );
  }
  for (const btn of document.querySelectorAll("#metrics-subtabs .subtab")) {
    btn.addEventListener("click", () =>
      setActiveTabImpl(ctx, `metrics/${btn.dataset.subtab}`, { refresh: false }),
    );
  }
  for (const btn of document.querySelectorAll("#run-detail-subtabs .subtab")) {
    btn.addEventListener("click", () => {
      ctx.setRunSubtab(btn.dataset.subtab);
      const path = `runs/${encodeURIComponent(ctx.getRunId() || "")}` +
        (ctx.getRunSubtab() !== "steps" ? `/${ctx.getRunSubtab()}` : "");
      setActiveTabImpl(ctx, path, { refresh: false });
      ctx.refreshDashboard();
    });
  }
  for (const btn of document.querySelectorAll("#audit-subtabs .subtab")) {
    btn.addEventListener("click", () => {
      ctx.setActiveAuditSubtabFromButton(btn.dataset.subtab);
      const newHash = ctx.buildAuditHash();
      if (window.location.hash !== newHash) {
        window.location.hash = newHash;
      } else {
        ctx.refreshDashboard();
      }
    });
  }
  for (const btn of document.querySelectorAll("#knowledge-subtabs .subtab")) {
    btn.addEventListener("click", () =>
      setActiveTabImpl(ctx, `knowledge/${btn.dataset.subtab}`, { refresh: false }),
    );
  }
  window.addEventListener("hashchange", () => {
    setActiveTabImpl(ctx, window.location.hash);
  });
  setActiveTabImpl(ctx, window.location.hash || "tasks", {
    refresh: false,
    updateHash: false,
  });
  ctx.refreshDashboard();
  setInterval(ctx.refreshDashboard, 30000);
}

// Public named exports (the stable import surface for app.js and future modules).
// These are the thin call-time wrappers; real work happens in *Impl against the ctx.
export function initRouter(ctx) {
  _routerCtx = ctx;
}

export function setActiveTab(raw, opts = {}) {
  const ctx = getCtx();
  return setActiveTabImpl(ctx, raw, opts);
}

export function navigateToRun(runId) {
  const ctx = getCtx();
  return navigateToRunImpl(ctx, runId);
}

export function initTabs() {
  const ctx = getCtx();
  return initTabsImpl(ctx);
}

export function setRunDetailSubtab(name) {
  const ctx = getCtx();
  setRunDetailSubtabImpl(ctx, name);
}

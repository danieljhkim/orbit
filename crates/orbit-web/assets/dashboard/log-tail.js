// Orbit dashboard log-tail panel (SSE stream, buffered logs, viewport resize, filters).
// Pure vanilla JS, split into ES modules with no build step.
//
// This module owns the #log-panel behavior on the Tasks tab. It is initialized
// by a single `initLogTail();` call from app.js (the bootstrap call site is kept
// in app.js per the extraction contract; the call still fires exactly once at
// page load). The module exports `initLogTail` and `fitLogPanelToViewport` for
// the two call sites that remain in app.js (`refreshDashboard` and `setActiveTab`).

import { el, fetchJson } from './common.js';

const $ = (id) => document.getElementById(id);

let logStream = null;
let logBuffered = [];
let logFollowTail = true;
let logRows = []; // Keep track to enforce max 200 after 250 limit
let activeLogFilters = new Set(["all"]);
let logPanelResizeWired = false;

// ORB-10874: the log pane can be collapsed and resized; both are local
// presentation preferences (not shared/synced state), so they persist to
// localStorage rather than the URL. `heightPx` is only present once the
// operator has dragged the handle — until then the panel keeps auto-fitting
// to the viewport exactly as before.
const LOG_PANEL_PREFS_KEY = "orbit.dashboard.logPanel";
const LOG_PANEL_MIN_HEIGHT = 160;

function loadLogPanelPrefs() {
  try {
    const raw = window.localStorage.getItem(LOG_PANEL_PREFS_KEY);
    const parsed = raw ? JSON.parse(raw) : {};
    return {
      collapsed: Boolean(parsed.collapsed),
      heightPx: Number.isFinite(parsed.heightPx) ? parsed.heightPx : null,
    };
  } catch (_) {
    return { collapsed: false, heightPx: null };
  }
}

function saveLogPanelPrefs(prefs) {
  try {
    window.localStorage.setItem(LOG_PANEL_PREFS_KEY, JSON.stringify(prefs));
  } catch (_) {
    /* localStorage unavailable (private mode, quota) — presentation prefs are non-essential */
  }
}

let logPanelPrefs = loadLogPanelPrefs();

function maxLogPanelHeight() {
  const panel = $("log-panel");
  if (!panel) return Infinity;
  const top = panel.getBoundingClientRect().top;
  return Math.max(LOG_PANEL_MIN_HEIGHT, Math.floor(window.innerHeight - top - 24));
}

export function fitLogPanelToViewport() {
  const panel = $("log-panel");
  if (!panel) return;
  applyLogPanelCollapsedState();
  if (logPanelPrefs.collapsed) return;
  const cap = maxLogPanelHeight();
  const height = logPanelPrefs.heightPx != null
    ? Math.min(Math.max(logPanelPrefs.heightPx, LOG_PANEL_MIN_HEIGHT), cap)
    : cap;
  panel.style.setProperty("--log-panel-height", `${height}px`);
}

function applyLogPanelCollapsedState() {
  const panel = $("log-panel");
  const toggle = $("log-panel-toggle");
  if (!panel) return;
  panel.classList.toggle("collapsed", logPanelPrefs.collapsed);
  if (toggle) {
    toggle.setAttribute("aria-expanded", logPanelPrefs.collapsed ? "false" : "true");
    toggle.textContent = logPanelPrefs.collapsed ? "expand" : "collapse";
  }
  if (logPanelPrefs.collapsed) {
    panel.style.removeProperty("--log-panel-height");
  }
}

function wireLogPanelToggle() {
  const toggle = $("log-panel-toggle");
  if (!toggle) return;
  toggle.addEventListener("click", () => {
    logPanelPrefs = { ...logPanelPrefs, collapsed: !logPanelPrefs.collapsed };
    saveLogPanelPrefs(logPanelPrefs);
    fitLogPanelToViewport();
  });
}

// A pointer-driven drag handle on the log panel's top edge. Dragging sets an
// explicit height (persisted), clamped so the panel can never be resized
// smaller than LOG_PANEL_MIN_HEIGHT nor larger than the viewport allows —
// the task list beside/above it keeps its own independent minimum height
// (dashboard.css), so this clamp only protects the log panel itself.
function wireLogPanelResizeHandle() {
  const handle = $("log-panel-resize");
  const panel = $("log-panel");
  if (!handle || !panel) return;

  let dragStartY = 0;
  let dragStartHeight = 0;

  const onPointerMove = (event) => {
    const delta = event.clientY - dragStartY;
    const cap = maxLogPanelHeight();
    const next = Math.min(Math.max(dragStartHeight + delta, LOG_PANEL_MIN_HEIGHT), cap);
    panel.style.setProperty("--log-panel-height", `${next}px`);
  };

  const onPointerUp = (event) => {
    document.removeEventListener("pointermove", onPointerMove);
    document.removeEventListener("pointerup", onPointerUp);
    const rect = panel.getBoundingClientRect();
    logPanelPrefs = { ...logPanelPrefs, heightPx: Math.round(rect.height), collapsed: false };
    saveLogPanelPrefs(logPanelPrefs);
    applyLogPanelCollapsedState();
  };

  handle.addEventListener("pointerdown", (event) => {
    if (logPanelPrefs.collapsed) return;
    event.preventDefault();
    dragStartY = event.clientY;
    dragStartHeight = panel.getBoundingClientRect().height;
    document.addEventListener("pointermove", onPointerMove);
    document.addEventListener("pointerup", onPointerUp);
  });

  // Keyboard resize (ArrowUp/ArrowDown) for operators who cannot drag.
  handle.addEventListener("keydown", (event) => {
    if (logPanelPrefs.collapsed) return;
    const step = 32;
    let delta = 0;
    if (event.key === "ArrowUp") delta = -step;
    else if (event.key === "ArrowDown") delta = step;
    else return;
    event.preventDefault();
    const cap = maxLogPanelHeight();
    const current = panel.getBoundingClientRect().height;
    const next = Math.min(Math.max(current + delta, LOG_PANEL_MIN_HEIGHT), cap);
    panel.style.setProperty("--log-panel-height", `${next}px`);
    logPanelPrefs = { ...logPanelPrefs, heightPx: Math.round(next), collapsed: false };
    saveLogPanelPrefs(logPanelPrefs);
  });
}

function wireLogPanelResize() {
  if (logPanelResizeWired) return;
  logPanelResizeWired = true;
  const scheduleFit = () => requestAnimationFrame(fitLogPanelToViewport);
  window.addEventListener("resize", scheduleFit);
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", scheduleFit);
  }
  wireLogPanelToggle();
  wireLogPanelResizeHandle();
  applyLogPanelCollapsedState();
}

function getLogClass(level, code) {
  if (code === "DENY") return "deny";
  if (code === "OK") return "ok";
  if (code === "ERR" || level === "error") return "err";
  if (code === "WRN" || level === "warn") return "warn";
  return "info";
}

function renderLogEvent(ev, isFresh) {
  const row = el("div", { class: "log-line" + (isFresh ? " fresh" : "") });
  row.dataset.code = ev.code || "";
  row.dataset.level = ev.level || "info";

  let timeStr = ev.ts || "";
  if (timeStr && timeStr.includes("T")) {
    const d = new Date(timeStr);
    if (!isNaN(d.getTime())) {
      timeStr = d.toLocaleTimeString("en-US", {hour12: false});
    }
  }

  const tSpan = el("span", { class: "t", text: timeStr });
  const agSpan = el("span", { class: "ag", text: ev.source || "" });
  const lvClass = getLogClass(ev.level, ev.code);
  const lvSpan = el("span", { class: `lv ${lvClass}`, text: ev.code || "" });
  const mSpan = el("span", { class: "m" });
  mSpan.innerHTML = ev.message_html || "";

  row.appendChild(tSpan);
  row.appendChild(agSpan);
  row.appendChild(lvSpan);
  row.appendChild(mSpan);

  // Click to expand/collapse the full message
  row.addEventListener("click", () => row.classList.toggle("expanded"));

  return row;
}

export function initLogTail() {
  wireLogPanelResize();
  fitLogPanelToViewport();
  fetchJson("/api/log?limit=50").then((events) => {
    const inner = $("logInner");
    if (!inner) return;
    inner.innerHTML = "";
    logRows = [];
    events.slice().reverse().forEach(ev => {
      const row = renderLogEvent(ev, false);
      inner.appendChild(row);
      logRows.push(row);
    });
    applyLogFilters();
    connectLogStream();
  }).catch(console.error);
  
  const followBtn = $("log-follow-tail");
  if (followBtn) {
    followBtn.addEventListener("click", (e) => {
      logFollowTail = !logFollowTail;
      e.currentTarget.classList.toggle("on", logFollowTail);
      if (logFollowTail) {
        flushBufferedLogs();
      }
    });
  }

  const btnBuffered = $("log-buffered-count");
  if (btnBuffered) {
    btnBuffered.addEventListener("click", () => {
      if (!logFollowTail) flushBufferedLogs();
    });
  }

  document.querySelectorAll("#log-panel .filter-pill").forEach(pill => {
    pill.addEventListener("click", (e) => {
      const filter = e.currentTarget.dataset.filter;
      if (filter === "all") {
        activeLogFilters.clear();
        activeLogFilters.add("all");
      } else {
        if (activeLogFilters.has("all")) {
          activeLogFilters.clear();
        }
        if (activeLogFilters.has(filter)) {
          activeLogFilters.delete(filter);
          if (activeLogFilters.size === 0) activeLogFilters.add("all");
        } else {
          activeLogFilters.add(filter);
        }
      }
      
      document.querySelectorAll("#log-panel .filter-pill").forEach(p => {
        p.classList.toggle("on", activeLogFilters.has(p.dataset.filter));
      });
      applyLogFilters();
    });
  });
}

function flushBufferedLogs() {
  const inner = $("logInner");
  if (!inner) return;
  const wasEmpty = logBuffered.length === 0;
  for (const ev of logBuffered) {
    const row = renderLogEvent(ev, true);
    inner.insertBefore(row, inner.firstChild);
    logRows.unshift(row);
    setTimeout(() => row.classList.remove("fresh"), 600);
  }
  logBuffered = [];
  const btnBuffered = $("log-buffered-count");
  if (btnBuffered) btnBuffered.style.display = "none";
  enforceLogBounds();
  if (!wasEmpty) applyLogFilters();
}

function enforceLogBounds() {
  if (logRows.length > 250) {
    const toRemove = logRows.splice(200);
    for (const row of toRemove) {
      row.remove();
    }
  }
}

function applyLogFilters() {
  let visibleCount = 0;
  for (const row of logRows) {
    const code = row.dataset.code;
    const level = row.dataset.level;
    const lvClass = getLogClass(level, code);
    
    let show = false;
    if (activeLogFilters.has("all")) {
      show = true;
    } else {
      if (activeLogFilters.has("err") && lvClass === "err") show = true;
      if (activeLogFilters.has("deny") && lvClass === "deny") show = true;
      if (activeLogFilters.has("warn") && lvClass === "warn") show = true;
    }
    row.style.display = show ? "" : "none";
    if (show) visibleCount++;
  }
  
  const cnt = $("log-count");
  if (cnt) cnt.textContent = `${visibleCount}`;
}

function connectLogStream() {
  if (logStream) logStream.close();
  logStream = new EventSource("/api/log/stream");
  logStream.onmessage = (e) => {
    try {
      const ev = JSON.parse(e.data);
      if (logFollowTail) {
        const inner = $("logInner");
        const row = renderLogEvent(ev, true);
        inner.insertBefore(row, inner.firstChild);
        logRows.unshift(row);
        applyLogFilters();
        enforceLogBounds();
        setTimeout(() => row.classList.remove("fresh"), 600);
      } else {
        logBuffered.push(ev);
        const btn = $("log-buffered-count");
        if (btn) {
          btn.textContent = `${logBuffered.length} buffered`;
          btn.style.display = "";
        }
      }
    } catch (err) {
      console.error("Failed to parse SSE event", err);
    }
  };
}

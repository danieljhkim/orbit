// Orbit dashboard diagnostics-domain (metrics + errors tables + implement_one side card).
// Pure vanilla JS, split into ES modules with no build step.
//
// lastDiagnostics and activeDiagSubtab live in app.js (mutated by the fetch
// closures in activeRefreshJobs, which are kept in app.js per scope). They are
// exposed read-only via the diagnosticsContext() factory in app.js, passed as
// the argument to render entry points. This mirrors the taskContext() /
// auditContext() pattern. Simpler getter approach wins here.
//
// Uses `el`, `syncNodes` from `./common.js` (re-defines $ locally, as other
// extracted modules do).
//
// Cross helpers (fmtRelative, fmtDuration, truncate, setActiveTab,
// navigateToRun) are provided via ctx. The row click uses setActiveTab
// (preserving the ?step= query for run-detail pre-expansion) rather than
// navigateToRun to ensure identical behavior to before the split.
//
// No behavior change. All original column shapes, truncation, click wiring,
// and side-card rendering preserved exactly.

import { el, syncNodes } from './common.js';

const $ = (id) => document.getElementById(id);

function hasCtx(ctx, key) {
  return ctx && typeof ctx[key] === "function";
}

function fmtRelativeValue(ctx, v) {
  return hasCtx(ctx, "fmtRelative") ? ctx.fmtRelative(v) : (v || "-");
}

function fmtDurationValue(ctx, v) {
  return hasCtx(ctx, "fmtDuration") ? ctx.fmtDuration(v) : (v == null ? "-" : String(v));
}

function truncateValue(ctx, s, n = 220) {
  return hasCtx(ctx, "truncate") ? ctx.truncate(s, n) : String(s || "").slice(0, n);
}

function getDiagMetricsColumns(ctx) {
  return [
    { key: "ts", label: "time", num: false, render: (v) => fmtRelativeValue(ctx, v) },
    { key: "step", label: "step", num: false },
    { key: "actor_identity", label: "actor", num: false, render: (v) => v || "-" },
    {
      key: "token_usage",
      label: "tokens",
      num: true,
      render: (v) => (v == null ? "-" : String(v)),
    },
    { key: "tool_invocations", label: "tools", num: true },
    {
      key: "step_duration_ms",
      label: "duration",
      num: true,
      render: (v) => fmtDurationValue(ctx, v),
    },
    { key: "retry_count", label: "retries", num: true },
  ];
}

function getDiagErrorsColumns(ctx) {
  return [
    { key: "ts", label: "time", num: false, render: (v) => fmtRelativeValue(ctx, v) },
    { key: "source", label: "source", num: false },
    { key: "provider", label: "provider", num: false, render: (v) => v || "-" },
    { key: "step", label: "step", num: false, render: (v) => v || "-" },
    {
      key: "message",
      label: "message",
      num: false,
      cellClass: "stderr",
      render: (v, row, td) => {
        const full = v || "";
        td.title = row.target ? `${row.target}: ${full}` : full;
        return truncateValue(ctx, full, 220);
      },
    },
  ];
}

function renderDiagnosticsTable(rows, columns, ctx) {
  const body = $("diag-body");
  
  if (!rows || rows.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No entries this month." })
    ])]);
    return;
  }
  
  let table = body.querySelector("table.scoreboard-table");
  let tbody;
  const tableSig = columns.map(c => c.key).join("-");
  if (!table || table.dataset.sig !== tableSig) {
    table = el("table", { class: "scoreboard-table" });
    table.dataset.sig = tableSig;
    const thead = el("thead");
    const headRow = el("tr");
    for (const col of columns) {
      headRow.appendChild(el("th", { class: col.num ? "num" : "", text: col.label }));
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
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const tr = el("tr");
    for (const col of columns) {
      const baseClass =
        (col.num ? "num" : "") + (col.cellClass ? ` ${col.cellClass}` : "");
      const td = el("td", { class: baseClass });
      const v = row[col.key];
      const text = col.render ? col.render(v, row, td) : v == null ? "" : String(v);
      td.textContent = text;
      tr.appendChild(td);
    }
    tr.dataset.key = `diag-${row.ts || ''}-${row.step || i}-${row.command || row.actor_identity || ''}`;
    tr.dataset.hash = JSON.stringify(row);
    if (row.job_run) {
      tr.classList.add("clickable");
      tr.title = "Open owning run";
      tr.addEventListener("click", () => {
        const stepQuery = row.step_index == null ? "" : `?step=${encodeURIComponent(row.step_index)}`;
        if (hasCtx(ctx, "setActiveTab")) {
          ctx.setActiveTab(`runs/${encodeURIComponent(row.job_run)}${stepQuery}`);
        }
      });
    }
    frag.appendChild(tr);
  }
  
  syncNodes(tbody, Array.from(frag.children));
}

function renderDiagnostics(ctx = {}) {
  const sub = ctx.getActiveDiagSubtab ? ctx.getActiveDiagSubtab() : "metrics";
  const last = ctx.getLastDiagnostics ? ctx.getLastDiagnostics() : { metrics: [], errors: [], implement_one: [] };
  const rows = last[sub] || [];
  $("diag-count").textContent = `${rows.length}`;
  const columns =
    sub === "metrics"
      ? getDiagMetricsColumns(ctx)
      : getDiagErrorsColumns(ctx);
  renderDiagnosticsTable(
    rows,
    columns,
    ctx,
  );

  const sidePanel = $("diagnostics-side-panel");
  if (sidePanel) {
    renderImplementOneCard($("diag-implement-one-body"), last.implement_one || [], ctx);
  }
}

function renderMetricsCard(container, title, rows, cols) {
  const card = el("div", { class: "audit-summary-card" });
  card.appendChild(el("div", { class: "card-title", text: title }));
  const body = el("div", { class: "card-body" });
  
  const table = el("table", { class: "summary-table" });
  const thead = el("thead");
  const tr = el("tr");
  for (const c of cols) tr.appendChild(el("th", { class: c.num ? "num" : "", text: c.label }));
  thead.appendChild(tr);
  table.appendChild(thead);

  const tbody = el("tbody");
  for (const item of rows) {
    const row = el("tr");
    for (const c of cols) {
      const val = c.format ? c.format(item[c.key]) : item[c.key];
      row.appendChild(el("td", { class: c.num ? "num" : "", text: val }));
    }
    tbody.appendChild(row);
  }
  table.appendChild(tbody);
  body.appendChild(table);
  card.appendChild(body);
  container.appendChild(card);
}

function renderImplementOneCard(container, rows, ctx = {}) {
  container.innerHTML = "";
  if (rows.length === 0) {
    container.appendChild(el("div", { class: "empty", text: "No implement_one runs in last 30d." }));
    return;
  }

  const durCols = [
    { key: "actor", label: "actor" },
    { key: "n", label: "n", num: true },
    { key: "avg", label: "avg", num: true, format: (v) => fmtDurationValue(ctx, v) },
    { key: "p50", label: "p50", num: true, format: (v) => fmtDurationValue(ctx, v) },
    { key: "p95", label: "p95", num: true, format: (v) => fmtDurationValue(ctx, v) }
  ];
  renderMetricsCard(container, "Average implement_one duration by actor (30d)", rows, durCols);
}

export {
  renderDiagnostics,
  renderImplementOneCard,
};

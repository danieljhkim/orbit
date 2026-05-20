// Orbit dashboard metrics-domain (CLI metrics parity tables + invocation filters).

import { el, fetchJson, syncNodes } from './common.js';

const $ = (id) => document.getElementById(id);

const DEFAULT_KNOWLEDGE_LIMIT = 100;
const DEFAULT_INVOCATION_LIMIT = 50;

let lastMetrics = {
  knowledge: null,
  activity: [],
  tools: [],
  task: null,
  invocations: [],
};

function activeSubtab(ctx) {
  return ctx && typeof ctx.getActiveMetricsSubtab === "function"
    ? ctx.getActiveMetricsSubtab()
    : "knowledge";
}

function showNode(id, show) {
  const node = $(id);
  if (node) node.style.display = show ? "" : "none";
}

function countFor(sub) {
  if (sub === "knowledge") {
    return lastMetrics.knowledge ? String(lastMetrics.knowledge.total_runs || 0) : "-";
  }
  if (sub === "activity") return String(lastMetrics.activity.length);
  if (sub === "tools") return String(lastMetrics.tools.length);
  if (sub === "task") return lastMetrics.task ? String(lastMetrics.task.invocation_count || 0) : "-";
  if (sub === "invocations") return String(lastMetrics.invocations.length);
  return "-";
}

function formatNumber(value, digits = 0) {
  const n = Number(value);
  if (!Number.isFinite(n)) return "-";
  if (!Number.isInteger(digits) || digits < 0 || digits > 20) digits = 0;
  return n.toLocaleString(undefined, {
    maximumFractionDigits: digits,
    minimumFractionDigits: digits,
  });
}

function formatDecimal(value) {
  return formatNumber(value, 1);
}

function formatPercent(value, digits = 1) {
  const n = Number(value);
  if (!Number.isFinite(n)) return "-";
  return `${(n * 100).toFixed(digits)}%`;
}

function formatRatio(value) {
  return value == null ? "-" : `${formatDecimal(value)}x`;
}

function formatTimestamp(ctx, value) {
  if (!value) return "-";
  return ctx && typeof ctx.fmtAbsTime === "function" ? ctx.fmtAbsTime(value) : String(value);
}

function taskCount(row) {
  return Array.isArray(row && row.task_ids) ? row.task_ids.length : 0;
}

function modelValue(value) {
  return value || "-";
}

function emptyState(text) {
  return el("div", { class: "empty-state" }, [
    el("div", { class: "icon", text: "*" }),
    el("div", { class: "text", text }),
  ]);
}

function tableCell(col, row, ctx) {
  const td = el("td", { class: col.num ? "num" : "" });
  const value = col.render ? col.render(row[col.key], row, ctx) : row[col.key];
  td.textContent = value == null ? "" : String(value);
  if (col.title) td.title = col.title(row[col.key], row, ctx) || "";
  return td;
}

function renderTable(rows, columns, emptyText, ctx) {
  if (!Array.isArray(rows) || rows.length === 0) {
    return emptyState(emptyText);
  }

  const table = el("table", { class: "scoreboard-table" });
  const thead = el("thead");
  const headRow = el("tr");
  for (const col of columns) {
    headRow.appendChild(el("th", { class: col.num ? "num" : "", text: col.label }));
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = el("tbody");
  rows.forEach((row, index) => {
    const tr = el("tr");
    for (const col of columns) tr.appendChild(tableCell(col, row, ctx));
    tr.dataset.key = `metrics-row-${index}-${JSON.stringify(row).slice(0, 80)}`;
    tr.dataset.hash = JSON.stringify(row);
    tbody.appendChild(tr);
  });
  table.appendChild(tbody);
  return table;
}

function kv(label, value, opts = {}) {
  return el("div", { class: "metrics-kv" }, [
    el("span", { class: "label", text: label }),
    el("span", {
      class: opts.accent ? "value accent" : "value",
      text: value == null || value === "" ? "-" : String(value),
      title: value == null ? "" : String(value),
    }),
  ]);
}

function summarySection(title, items) {
  return el("section", { class: "metrics-summary-section" }, [
    el("h3", { text: title }),
    el("div", { class: "metrics-kv-grid" }, items),
  ]);
}

function renderKnowledge(summary) {
  if (!summary || Number(summary.total_runs || 0) === 0) {
    return emptyState("No knowledge usage metrics found.");
  }

  const compression = summary.compression || {};
  const doubleRead = summary.double_read || {};
  const input = summary.total_llm_input_tokens || {};
  return el("div", { class: "metrics-summary" }, [
    summarySection("Runs", [
      kv("total", formatNumber(summary.total_runs), { accent: true }),
      kv("with pack", formatNumber(summary.pack_runs)),
      kv("fallback", formatNumber(summary.fallback_runs)),
      kv("fallback rate", formatPercent(summary.fallback_rate), { accent: true }),
    ]),
    summarySection("Compression (tokenized, cl100k_base)", [
      kv("mean", formatRatio(compression.mean), { accent: true }),
      kv("p50", formatRatio(compression.p50)),
      kv("p90", formatRatio(compression.p90)),
      kv("min", formatRatio(compression.min)),
    ]),
    summarySection("Double-read guard", [
      kv("mean rate", `${formatNumber(doubleRead.mean_rate, 2)} (${formatPercent(doubleRead.mean_rate, 0)} baseline re-read)`, { accent: true }),
      kv("runs >50%", `${formatNumber(doubleRead.runs_over_fifty_percent)} / ${formatNumber(doubleRead.measured_runs)}`),
    ]),
    summarySection("Total LLM input tokens per activity", [
      kv("with pack avg", input.with_pack_avg == null ? "-" : `${formatNumber(input.with_pack_avg)} tokens`, { accent: true }),
      kv("without pack avg", input.without_pack_avg == null ? "-" : `${formatNumber(input.without_pack_avg)} tokens`),
      kv("estimated savings", input.estimated_savings == null ? "-" : formatPercent(input.estimated_savings, 0)),
    ]),
  ]);
}

function activityColumns() {
  return [
    { key: "activity_id", label: "activity" },
    { key: "agent", label: "agent" },
    { key: "model", label: "model", render: modelValue },
    { key: "invocation_count", label: "invocations", num: true, render: formatNumber },
    { key: "avg_tokens", label: "avg", num: true, render: formatDecimal },
    { key: "p50_tokens", label: "p50", num: true, render: formatNumber },
    { key: "p95_tokens", label: "p95", num: true, render: formatNumber },
    { key: "total_tokens", label: "total", num: true, render: formatNumber },
    { key: "total_input_tokens", label: "input", num: true, render: formatNumber },
    { key: "total_cache_read_tokens", label: "cache read", num: true, render: formatNumber },
    { key: "total_cache_create_tokens", label: "cache create", num: true, render: formatNumber },
    { key: "total_output_tokens", label: "output", num: true, render: formatNumber },
    { key: "total_tool_calls", label: "tools", num: true, render: formatNumber },
  ];
}

function toolColumns() {
  return [
    { key: "activity_id", label: "activity" },
    { key: "tool_name", label: "tool" },
    { key: "call_count", label: "calls", num: true, render: formatNumber },
    { key: "avg_result_bytes", label: "avg result bytes", num: true, render: formatDecimal },
    { key: "total_result_bytes", label: "total result bytes", num: true, render: formatNumber },
  ];
}

function taskColumns() {
  return [
    { key: "task_id", label: "task" },
    { key: "invocation_count", label: "invocations", num: true, render: formatNumber },
    { key: "total_tokens", label: "total", num: true, render: formatNumber },
    { key: "total_input_tokens", label: "input", num: true, render: formatNumber },
    { key: "total_cache_read_tokens", label: "cache read", num: true, render: formatNumber },
    { key: "total_cache_create_tokens", label: "cache create", num: true, render: formatNumber },
    { key: "total_output_tokens", label: "output", num: true, render: formatNumber },
    { key: "total_tool_calls", label: "tools", num: true, render: formatNumber },
  ];
}

function invocationColumns() {
  return [
    { key: "ts", label: "ts", render: (value, row, ctx) => formatTimestamp(ctx, value) },
    { key: "job_run_id", label: "job run" },
    { key: "activity_id", label: "activity" },
    { key: "agent", label: "agent" },
    { key: "model", label: "model", render: modelValue },
    { key: "total_tokens", label: "total", num: true, render: formatNumber },
    { key: "input_tokens", label: "input", num: true, render: formatNumber },
    { key: "cache_read_tokens", label: "cache read", num: true, render: formatNumber },
    { key: "cache_create_tokens", label: "cache create", num: true, render: formatNumber },
    { key: "output_tokens", label: "output", num: true, render: formatNumber },
    { key: "tool_call_count", label: "tools", num: true, render: formatNumber },
    { key: "task_ids", label: "tasks", num: true, render: (_value, row) => formatNumber(taskCount(row)) },
  ];
}

function taskIdFromForm() {
  return ($("metrics-task-id")?.value || "").trim();
}

function setTaskId(value) {
  const input = $("metrics-task-id");
  if (input && value) input.value = value;
}

function invocationsParams() {
  const fields = [
    ["since", "metrics-filter-since"],
    ["until", "metrics-filter-until"],
    ["job_run_id", "metrics-filter-job-run-id"],
    ["activity_id", "metrics-filter-activity-id"],
    ["task_id", "metrics-filter-task-id"],
    ["agent", "metrics-filter-agent"],
    ["model", "metrics-filter-model"],
    ["tool_name", "metrics-filter-tool-name"],
    ["limit", "metrics-filter-limit"],
  ];
  const sp = new URLSearchParams();
  for (const [key, id] of fields) {
    const value = ($(id)?.value || "").trim();
    if (value) sp.set(key, value);
  }
  if (!sp.has("limit")) sp.set("limit", String(DEFAULT_INVOCATION_LIMIT));
  return sp;
}

function inferTaskId() {
  const current = taskIdFromForm();
  if (current) return Promise.resolve(current);
  const fromRows = lastMetrics.invocations.find((row) => Array.isArray(row.task_ids) && row.task_ids[0]);
  if (fromRows) {
    setTaskId(fromRows.task_ids[0]);
    return Promise.resolve(fromRows.task_ids[0]);
  }
  return fetchJson("/api/metrics/invocations?limit=1").then((rows) => {
    const id = Array.isArray(rows) && rows[0] && Array.isArray(rows[0].task_ids)
      ? rows[0].task_ids[0]
      : "";
    if (id) setTaskId(id);
    return id || "";
  });
}

function renderTask() {
  return renderTable(
    lastMetrics.task ? [lastMetrics.task] : [],
    taskColumns(),
    "No task metrics found.",
  );
}

function renderMetrics(ctx = {}) {
  const sub = activeSubtab(ctx);
  const body = $("metrics-body");
  if (!body) return;

  showNode("metrics-task-controls", sub === "task");
  showNode("metrics-invocation-controls", sub === "invocations");
  const title = $("metrics-title");
  if (title) title.textContent = `Metrics / ${sub}`;
  const count = $("metrics-count");
  if (count) count.textContent = countFor(sub);

  let node;
  if (sub === "knowledge") {
    node = renderKnowledge(lastMetrics.knowledge);
  } else if (sub === "activity") {
    node = renderTable(lastMetrics.activity, activityColumns(), "No invocation metrics found.", ctx);
  } else if (sub === "tools") {
    node = renderTable(lastMetrics.tools, toolColumns(), "No tool call metrics found.", ctx);
  } else if (sub === "task") {
    node = renderTask();
  } else {
    node = renderTable(lastMetrics.invocations, invocationColumns(), "No invocation records found.", ctx);
  }
  node.dataset.key = `metrics-${sub}`;
  node.dataset.hash = JSON.stringify(lastMetrics[sub] || null);
  syncNodes(body, [node]);
}

function fetchAndRenderMetrics(ctx = {}) {
  const sub = activeSubtab(ctx);
  if (sub === "knowledge") {
    return fetchJson(`/api/metrics/knowledge?limit=${DEFAULT_KNOWLEDGE_LIMIT}`).then((payload) => {
      lastMetrics.knowledge = payload;
      renderMetrics(ctx);
    });
  }
  if (sub === "activity") {
    return fetchJson("/api/metrics/activity").then((rows) => {
      lastMetrics.activity = Array.isArray(rows) ? rows : [];
      renderMetrics(ctx);
    });
  }
  if (sub === "tools") {
    return fetchJson("/api/metrics/tools").then((rows) => {
      lastMetrics.tools = Array.isArray(rows) ? rows : [];
      renderMetrics(ctx);
    });
  }
  if (sub === "task") {
    return inferTaskId().then((taskId) => {
      if (!taskId) {
        lastMetrics.task = null;
        renderMetrics(ctx);
        return null;
      }
      return fetchJson(`/api/metrics/task/${encodeURIComponent(taskId)}`).then((row) => {
        lastMetrics.task = row;
        renderMetrics(ctx);
      });
    });
  }

  return fetchJson(`/api/metrics/invocations?${invocationsParams().toString()}`).then((rows) => {
    lastMetrics.invocations = Array.isArray(rows) ? rows : [];
    renderMetrics(ctx);
  });
}

function initMetrics(ctx = {}) {
  const taskForm = $("metrics-task-form");
  if (taskForm) {
    taskForm.addEventListener("submit", (event) => {
      event.preventDefault();
      fetchAndRenderMetrics(ctx).catch(console.error);
    });
  }
  const invocationsForm = $("metrics-invocation-form");
  if (invocationsForm) {
    invocationsForm.addEventListener("submit", (event) => {
      event.preventDefault();
      fetchAndRenderMetrics(ctx).catch(console.error);
    });
  }
}

export {
  fetchAndRenderMetrics,
  initMetrics,
  renderMetrics,
};

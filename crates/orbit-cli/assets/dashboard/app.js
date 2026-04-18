// Orbit dashboard — terminal-dark, polling-based read-only SPA.
// Pure vanilla JS, no build step. Polls every POLL_MS or ?poll=<ms>.

const STATUS_ORDER = [
  "in-progress",
  "review",
  "blocked",
  "proposed",
  "backlog",
  "someday",
  "done",
  "rejected",
  "archived",
];

const DEFAULT_INACTIVE_STATUSES = new Set(["done", "someday", "archived"]);

const params = new URLSearchParams(window.location.search);
const POLL_MS = Math.max(1000, parseInt(params.get("poll") || "5000", 10));

const $ = (id) => document.getElementById(id);

let searchQuery = "";
let activeStatuses = new Set(
  STATUS_ORDER.filter((s) => !DEFAULT_INACTIVE_STATUSES.has(s)),
);
let lastTasks = [];
let lastDiagnostics = { metrics: [], friction: [] };
let activeDiagSubtab = "metrics";
let expandedTaskIds = new Set();

function el(tag, opts = {}, children = []) {
  const node = document.createElement(tag);
  if (opts.class) node.className = opts.class;
  if (opts.text != null) node.textContent = opts.text;
  if (opts.title != null) node.title = opts.title;
  if (opts.style) Object.assign(node.style, opts.style);
  for (const child of children) {
    if (child == null) continue;
    node.appendChild(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
}

function statusPill(status) {
  const color = `var(--status-${status}, var(--fg))`;
  const pill = el("span", { class: "pill mono", text: status });
  pill.style.color = color;
  pill.style.borderLeft = `2px solid ${color}`;
  return pill;
}

function priorityCell(p) {
  const node = el("span", { class: "priority mono", text: p });
  node.style.color = `var(--priority-${p}, var(--fg-dim))`;
  return node;
}

function stateCell(state) {
  const node = el("span", { class: "mono", text: state });
  node.style.color = `var(--state-${state}, var(--fg-dim))`;
  return node;
}

function fetchJson(path) {
  return fetch(path, { headers: { accept: "application/json" } })
    .then(res => {
      if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
      return res.json();
    });
}

function syncNodes(container, newNodesArr) {
  const oldNodes = Array.from(container.children);
  const oldMap = new Map();
  for (const node of oldNodes) {
    if (node.dataset.key) oldMap.set(node.dataset.key, node);
  }

  for (let i = 0; i < newNodesArr.length; i++) {
    const newNode = newNodesArr[i];
    const key = newNode.dataset.key;
    let nodeToPlace = newNode;

    if (key && oldMap.has(key)) {
      const oldNode = oldMap.get(key);
      if (oldNode.dataset.hash === newNode.dataset.hash) {
        nodeToPlace = oldNode;
      } else {
        nodeToPlace.classList.add("data-changed");
      }
    } else if (key) {
      nodeToPlace.classList.add("data-new");
    }

    if (container.children[i] !== nodeToPlace) {
      if (container.children[i]) {
        container.insertBefore(nodeToPlace, container.children[i]);
      } else {
        container.appendChild(nodeToPlace);
      }
    }
  }

  while (container.children.length > newNodesArr.length) {
    container.removeChild(container.lastElementChild);
  }
}

function filterTasks(tasks) {
  const q = searchQuery;
  return tasks.filter((t) => {
    if (!activeStatuses.has(t.status)) return false;
    if (!q) return true;
    return (
      (t.id && t.id.toLowerCase().includes(q)) ||
      (t.title && t.title.toLowerCase().includes(q))
    );
  });
}

const TASK_META_FIELDS = [
  ["implemented_by", "implemented_by"],
  ["planned_by", "planned_by"],
  ["created_by", "created_by"],
  ["pr_number", "pr"],
  ["pr_status", "pr_status"],
  ["created_at", "created"],
  ["updated_at", "updated"],
];

function buildTaskDetail(task) {
  const detail = el("div", { class: "row-detail" });
  detail.addEventListener("click", (e) => e.stopPropagation());

  const meta = el("div", { class: "meta-line" });
  let metaCount = 0;
  for (const [key, label] of TASK_META_FIELDS) {
    const v = task[key];
    if (v == null || v === "") continue;
    const display = key.endsWith("_at") ? fmtAbsTime(v) : String(v);
    const span = el("span", {}, [
      el("span", { class: "label", text: `${label}:` }),
      el("span", { class: "value", text: display }),
    ]);
    meta.appendChild(span);
    metaCount++;
  }
  if (metaCount > 0) detail.appendChild(meta);

  if (task.description && task.description.trim()) {
    detail.appendChild(el("h4", { text: "description" }));
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.description);
    } else {
      view.textContent = task.description;
    }
    detail.appendChild(view);
  }

  if (Array.isArray(task.acceptance_criteria) && task.acceptance_criteria.length > 0) {
    detail.appendChild(el("h4", { text: "acceptance criteria" }));
    const ul = el("ul", { class: "ac-list" });
    for (const ac of task.acceptance_criteria) {
      if (typeof marked !== "undefined") {
        const li = el("li");
        li.innerHTML = marked.parseInline(ac);
        ul.appendChild(li);
      } else {
        ul.appendChild(el("li", { text: ac }));
      }
    }
    detail.appendChild(ul);
  }

  if (task.plan && task.plan.trim()) {
    detail.appendChild(el("h4", { text: "plan" }));
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.plan);
    } else {
      view.textContent = task.plan;
    }
    detail.appendChild(view);
  }

  if (task.execution_summary && task.execution_summary.trim()) {
    detail.appendChild(el("h4", { text: "execution summary" }));
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.execution_summary);
    } else {
      view.textContent = task.execution_summary;
    }
    detail.appendChild(view);
  }

  if (Array.isArray(task.comments) && task.comments.length > 0) {
    detail.appendChild(el("h4", { text: "comments" }));
    for (const c of task.comments) {
      const line = el("div", { class: "comment-line" }, [
        document.createTextNode(`[${fmtAbsTime(c.at)}] `),
        el("span", { class: "author", text: c.by || "?" }),
        document.createTextNode(`: ${c.message || ""}`),
      ]);
      detail.appendChild(line);
    }
  }

  if (Array.isArray(task.context_files) && task.context_files.length > 0) {
    detail.appendChild(el("h4", { text: "context" }));
    const ul = el("ul", { class: "file-list" });
    for (const path of task.context_files) {
      ul.appendChild(el("li", { text: path }));
    }
    detail.appendChild(ul);
  }

  if (Array.isArray(task.history) && task.history.length > 0) {
    detail.appendChild(el("h4", { text: "recent history" }));
    const recent = task.history.slice(-5).reverse();
    for (const h of recent) {
      const note = h.note ? ` (${h.note})` : "";
      const line = el("div", { class: "history-line" }, [
        document.createTextNode(`[${fmtAbsTime(h.at)}] `),
        el("span", { class: "actor", text: h.by || "?" }),
        document.createTextNode(`: ${h.event}${note}`),
      ]);
      detail.appendChild(line);
    }
  }

  detail.appendChild(buildActionsRow(task, detail));

  return detail;
}

const APPROVE_STATUSES = new Set(["proposed", "review"]);
const REJECT_STATUSES = new Set(["proposed", "review", "backlog"]);

function buildActionsRow(task, detail) {
  const actions = el("div", { class: "actions" });
  if (APPROVE_STATUSES.has(task.status)) {
    const btn = el("button", { class: "action approve", text: "approve" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      runAction(task, "approve", detail, null, btn);
    });
    actions.appendChild(btn);
  }
  if (REJECT_STATUSES.has(task.status)) {
    const btn = el("button", { class: "action reject", text: "reject" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      showRejectForm(task, detail, actions);
    });
    actions.appendChild(btn);
  }
  if (task.status !== "archived") {
    const btn = el("button", { class: "action archive", text: "archive" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (window.confirm(`Archive task ${task.id}?`)) {
        runAction(task, "archive", detail, null, btn);
      }
    });
    actions.appendChild(btn);
  }
  return actions;
}

function showRejectForm(task, detail, actions) {
  const form = el("div", { class: "reject-form" });
  form.addEventListener("click", (e) => e.stopPropagation());
  const ta = el("textarea");
  ta.placeholder = "reason for rejection";
  const buttons = el("div", { class: "actions" });
  const submit = el("button", { class: "action reject", text: "submit" });
  const cancel = el("button", { class: "action cancel", text: "cancel" });
  submit.addEventListener("click", (e) => {
    e.stopPropagation();
    const note = ta.value.trim();
    if (!note) {
      ta.focus();
      return;
    }
    runAction(task, "reject", detail, { note }, submit);
  });
  cancel.addEventListener("click", (e) => {
    e.stopPropagation();
    form.replaceWith(actions);
  });
  buttons.appendChild(submit);
  buttons.appendChild(cancel);
  form.appendChild(ta);
  form.appendChild(buttons);
  actions.replaceWith(form);
  ta.focus();
}

async function runAction(task, kind, detail, body, btnNode) {
  // Disable buttons while in flight to prevent double-clicks
  for (const b of detail.querySelectorAll("button.action")) b.disabled = true;
  let oldText = "";
  if (btnNode) {
    oldText = btnNode.textContent;
    btnNode.innerHTML = `<span class="spinner"></span>wait`;
  }
  // Clear any prior error
  const prior = detail.querySelector(".action-error");
  if (prior) prior.remove();
  try {
    const res = await fetch(`/api/tasks/${encodeURIComponent(task.id)}/${kind}`, {
      method: "POST",
      headers: body ? { "content-type": "application/json" } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      let msg = `${kind} failed: HTTP ${res.status}`;
      try {
        const errBody = await res.json();
        if (errBody && errBody.error) msg = `${kind} failed: ${errBody.error}`;
      } catch (_) {
        /* keep generic msg */
      }
      throw new Error(msg);
    }
    expandedTaskIds.delete(task.id);
    await tick();
  } catch (err) {
    for (const b of detail.querySelectorAll("button.action")) b.disabled = false;
    if (btnNode) btnNode.textContent = oldText;
    const errEl = el("div", { class: "action-error", text: String(err.message || err) });
    detail.prepend(errEl);
  }
}

function renderTasks(tasks) {
  const body = $("tasks-body");
  const frag = document.createDocumentFragment();
  
  const filtered = filterTasks(tasks);
  $("tasks-count").textContent =
    filtered.length === tasks.length
      ? `${tasks.length}`
      : `${filtered.length}/${tasks.length}`;
  if (filtered.length === 0) {
    const defaultText = tasks.length === 0 ? "No tasks available." : "No tasks match filter.";
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: defaultText })
    ])]);
    return;
  }
  const groups = new Map();
  for (const t of filtered) {
    if (!groups.has(t.status)) groups.set(t.status, []);
    groups.get(t.status).push(t);
  }
  const ordered = STATUS_ORDER.filter((s) => groups.has(s)).concat(
    [...groups.keys()].filter((s) => !STATUS_ORDER.includes(s)),
  );
  for (const status of ordered) {
    const group = groups.get(status);
    const header = el("div", { class: "group-header" }, [
      statusPill(status),
      el("span", { class: "group-count", text: `${group.length}` }),
    ]);
    header.dataset.key = `header-${status}`;
    header.dataset.hash = `${status}-${group.length}`;
    frag.appendChild(header);
    for (const t of group) {
      const idSpan = el("span", { class: "id mono", text: t.id, title: "Click to copy ID" });
      idSpan.addEventListener("click", (e) => {
        e.stopPropagation();
        navigator.clipboard.writeText(t.id).catch(() => {});
        const oldText = idSpan.textContent;
        idSpan.textContent = "copied!";
        idSpan.style.color = "var(--state-success)";
        setTimeout(() => {
          idSpan.textContent = oldText;
          idSpan.style.color = "";
        }, 1000);
      });
      const row = el("div", { class: "row", title: t.title }, [
        idSpan,
        el("span", { class: "title", text: t.title }),
        priorityCell(t.priority),
        el("span", { class: "type mono", text: t.type }),
      ]);
      row.dataset.key = `task-${t.id}`;
      // Basic hash based on row presentation parameters + expanded state
      row.dataset.hash = `${t.id}-${t.title}-${t.priority}-${t.type}-${expandedTaskIds.has(t.id)}`;
      row.addEventListener("click", () => {
        const toggle = () => {
          if (expandedTaskIds.has(t.id)) expandedTaskIds.delete(t.id);
          else expandedTaskIds.add(t.id);
          renderTasks(lastTasks);
        };
        if (document.startViewTransition) {
          row.style.viewTransitionName = `task-row-${t.id}`;
          document.startViewTransition(toggle).finished.then(() => {
            row.style.viewTransitionName = "";
          });
        } else {
          toggle();
        }
      });
      if (expandedTaskIds.has(t.id)) row.classList.add("expanded");
      frag.appendChild(row);
      if (expandedTaskIds.has(t.id)) {
        const detail = buildTaskDetail(t);
        detail.dataset.key = `detail-${t.id}`;
        // Diff by full task object stringified
        detail.dataset.hash = JSON.stringify(t);
        frag.appendChild(detail);
      }
    }
  }
  syncNodes(body, Array.from(frag.children));
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
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m${Math.floor((ms % 60000) / 1000)}s`;
}

function renderRuns(runs) {
  const body = $("runs-body");
  const frag = document.createDocumentFragment();
  
  const top = runs.slice(0, 20);
  $("runs-count").textContent = `${top.length}/${runs.length}`;
  if (top.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No job runs yet." })
    ])]);
    return;
  }
  for (const r of top) {
    const ts = r.finished_at || r.started_at || r.scheduled_at || r.created_at;
    const row = el("div", { class: "runs-row", title: r.run_id }, [
      el("span", { class: "when", text: fmtTimestamp(ts) }),
      el("span", { class: "id", text: r.job_id }),
      el("span", { class: "duration", text: fmtDuration(r.duration_ms) }),
      el("span", { class: "state" }, [stateCell(r.state)]),
    ]);
    row.dataset.key = `run-${r.run_id}`;
    row.dataset.hash = `${r.run_id}-${ts}-${r.duration_ms}-${r.state}`;
    frag.appendChild(row);
  }
  syncNodes(body, Array.from(frag.children));
}

const SCOREBOARD_COLUMNS = [
  { key: "agent", label: "agent", num: false },
  { key: "tasks_completed", label: "tasks", num: true },
  { key: "tokens.total", label: "tokens", num: true },
  { key: "tokens.output", label: "out", num: true },
  { key: "tool_calls", label: "tools", num: true },
  { key: "duels.wins", label: "duel w", num: true },
  { key: "duels.losses", label: "duel l", num: true },
  { key: "friction.reported", label: "frict r", num: true },
  { key: "friction.accepted", label: "frict a", num: true },
  { key: "friction.rejected", label: "frict rej", num: true },
  { key: "pr.merged_clean", label: "pr clean", num: true },
  { key: "pr.merged_with_revision", label: "pr w/rev", num: true },
  { key: "pr.review_comments", label: "pr cmts", num: true },
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
  $("scoreboard-count").textContent = `${entries.length}`;
  
  if (entries.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No scoreboard data yet." })
    ])]);
    return;
  }
  
  entries.sort(([, a], [, b]) => (b.tasks_completed || 0) - (a.tasks_completed || 0));

  let table = body.querySelector("table.scoreboard-table");
  let tbody;
  if (!table) {
    table = el("table", { class: "scoreboard-table" });
    const thead = el("thead");
    const headRow = el("tr");
    for (const col of SCOREBOARD_COLUMNS) {
      headRow.appendChild(
        el(col === SCOREBOARD_COLUMNS[0] ? "th" : "th", {
          class: col.num ? "num" : "",
          text: col.label,
        }),
      );
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
  for (const [name, agent] of entries) {
    const row = el("tr");
    for (const col of SCOREBOARD_COLUMNS) {
      let cellText;
      let extra = "";
      if (col.key === "agent") {
        cellText = name;
      } else {
        const v = readPath(agent, col.key);
        const num = typeof v === "number" ? v : 0;
        cellText = String(num);
        if (num === 0) extra = " zero";
      }
      const cellClass =
        (col.num ? "num" : col.key === "agent" ? "agent" : "") + extra;
      row.appendChild(
        el("td", {
          class: cellClass,
          text: cellText,
          title: col.key === "agent" ? name : undefined,
        }),
      );
    }
    row.dataset.key = `agent-${name}`;
    row.dataset.hash = JSON.stringify(agent);
    frag.appendChild(row);
  }
  
  syncNodes(tbody, Array.from(frag.children));
}

function refreshChips() {
  for (const chip of document.querySelectorAll("#task-filter .chip")) {
    const status = chip.dataset.status;
    const isAll = chip.dataset.role === "all";
    const allOn = activeStatuses.size === STATUS_ORDER.length;
    const on = isAll ? allOn : activeStatuses.has(status);
    chip.classList.toggle("active", on);
  }
}

function buildChips() {
  const container = $("task-filter");
  container.innerHTML = "";
  const allChip = el("button", { class: "chip", text: "all" });
  allChip.dataset.role = "all";
  allChip.addEventListener("click", () => {
    activeStatuses = new Set(STATUS_ORDER);
    refreshChips();
    renderTasks(lastTasks);
  });
  container.appendChild(allChip);
  for (const status of STATUS_ORDER) {
    const chip = el("button", { class: "chip", text: status });
    chip.dataset.status = status;
    chip.style.borderLeft = `2px solid var(--status-${status}, var(--border))`;
    chip.addEventListener("click", () => {
      if (activeStatuses.has(status)) {
        activeStatuses.delete(status);
      } else {
        activeStatuses.add(status);
      }
      refreshChips();
      renderTasks(lastTasks);
    });
    container.appendChild(chip);
  }
  refreshChips();
}

function wireSearch() {
  $("task-search").addEventListener("input", (e) => {
    searchQuery = e.target.value.trim().toLowerCase();
    renderTasks(lastTasks);
  });
}

const TABS = ["tasks", "scoreboard", "diagnostics"];
const DIAG_SUBTABS = ["metrics", "friction"];

      function setActiveTab(raw) {
  const [topRaw, subRaw] = String(raw || "").split("/");
  const top = TABS.includes(topRaw) ? topRaw : "tasks";
  for (const tab of document.querySelectorAll(".tab")) {
    tab.classList.toggle("active", tab.dataset.tab === top);
  }
  for (const pane of document.querySelectorAll(".tab-pane")) {
    pane.classList.toggle("active", pane.dataset.tab === top);
  }
  
  const indicator = $("tab-indicator") || el("div", {id: "tab-indicator", class: "tab-indicator"});
  if (!indicator.parentNode) document.querySelector(".tabs").appendChild(indicator);
  const activeTab = document.querySelector(`.tab[data-tab="${top}"]`);
  if (activeTab) {
    indicator.style.width = `${activeTab.offsetWidth}px`;
    indicator.style.left = `${activeTab.offsetLeft}px`;
  }

  let hash = `#${top}`;
  if (top === "diagnostics") {
    const sub = DIAG_SUBTABS.includes(subRaw) ? subRaw : activeDiagSubtab;
    setDiagSubtab(sub);
    hash = `#diagnostics/${sub}`;
  }
  if (window.location.hash !== hash) {
    window.location.hash = hash;
  }
}

function setDiagSubtab(name) {
  if (!DIAG_SUBTABS.includes(name)) name = "metrics";
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

  $("diag-title").textContent = name;
  renderDiagnostics();
}

function initTabs() {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.addEventListener("click", () => setActiveTab(tab.dataset.tab));
  }
  for (const btn of document.querySelectorAll("#diag-subtabs .subtab")) {
    btn.addEventListener("click", () => setActiveTab(`diagnostics/${btn.dataset.subtab}`));
  }
  window.addEventListener("hashchange", () => {
    setActiveTab(window.location.hash.replace(/^#/, ""));
  });
  setActiveTab(window.location.hash.replace(/^#/, "") || "tasks");
}

function fmtRelative(iso) {
  return fmtTimestamp(iso);
}

function truncate(text, max) {
  if (text == null) return "";
  if (text.length <= max) return text;
  return text.slice(0, max) + "\u2026";
}

const DIAG_METRICS_COLUMNS = [
  { key: "ts", label: "time", num: false, render: (v) => fmtRelative(v) },
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
    render: (v) => (v == null ? "-" : fmtDuration(v)),
  },
  { key: "retry_count", label: "retries", num: true },
];

const DIAG_FRICTION_COLUMNS = [
  { key: "ts", label: "time", num: false, render: (v) => fmtRelative(v) },
  { key: "step", label: "step", num: false },
  { key: "command", label: "command", num: false },
  {
    key: "exit_code",
    label: "exit",
    num: true,
    render: (v, _row, td) => {
      if (v == null) return "-";
      if (v !== 0) td.classList.add("exit-fail");
      return String(v);
    },
  },
  {
    key: "stderr",
    label: "stderr",
    num: false,
    cellClass: "stderr",
    render: (v, _row, td) => {
      const full = v || "";
      td.title = full;
      return truncate(full, 160);
    },
  },
];

function renderDiagnosticsTable(rows, columns) {
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
    frag.appendChild(tr);
  }
  
  syncNodes(tbody, Array.from(frag.children));
}

function renderDiagnostics() {
  const sub = activeDiagSubtab;
  const rows = lastDiagnostics[sub] || [];
  $("diag-count").textContent = `${rows.length}`;
  renderDiagnosticsTable(
    rows,
    sub === "metrics" ? DIAG_METRICS_COLUMNS : DIAG_FRICTION_COLUMNS,
  );
}

async function tick() {
  const now = new Date();
  $("meta-text").textContent = `fetching...`;
  $("conn-status").className = "status-dot orange";
  
  let hasErrors = false;
  
  await Promise.all([
    fetchJson("/api/tasks")
      .then((tasks) => {
        lastTasks = tasks;
        renderTasks(tasks);
      })
      .catch((e) => { hasErrors = true; console.error(e); }),
    fetchJson("/api/job-runs").then(renderRuns).catch((e) => { hasErrors = true; console.error(e); }),
    fetchJson("/api/scoreboard")
      .then(renderScoreboard)
      .catch((e) => { hasErrors = true; console.error(e); }),
    fetchJson("/api/diagnostics/metrics")
      .then((rows) => {
        lastDiagnostics.metrics = rows;
        if (activeDiagSubtab === "metrics") renderDiagnostics();
      })
      .catch((e) => { hasErrors = true; console.error(e); }),
    fetchJson("/api/diagnostics/friction")
      .then((rows) => {
        lastDiagnostics.friction = rows;
        if (activeDiagSubtab === "friction") renderDiagnostics();
      })
      .catch((e) => { hasErrors = true; console.error(e); }),
  ]);
  
  if (hasErrors) {
    $("conn-status").className = "status-dot red";
    $("meta-text").textContent = `offline · ${now.toLocaleTimeString()}`;
  } else {
    $("conn-status").className = "status-dot green";
    $("meta-text").textContent = `polled ${now.toLocaleTimeString()} · ${POLL_MS}ms`;
  }
  
  $("footer").textContent = `orbit dashboard · GET /api/{tasks,jobs,job-runs,audit,scoreboard,diagnostics/{metrics,friction}}`;
}

initTabs();
buildChips();
wireSearch();
tick();
setInterval(tick, POLL_MS);

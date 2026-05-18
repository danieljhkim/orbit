// Orbit dashboard — terminal-dark, manually refreshed SPA.
// Pure vanilla JS, no build step.

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

const params = new URLSearchParams(window.location.search);
function positiveIntParam(name, fallback) {
  const parsed = parseInt(params.get(name) || String(fallback), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
const JOB_RUN_LIMIT = positiveIntParam("runs", 25);
const DIAG_LIMIT = positiveIntParam("diag", 50);
const AUDIT_LIMIT = positiveIntParam("audit", 50);
const RUN_EVENTS_LIMIT = positiveIntParam("events", 100);
const LEARNING_LIMIT = positiveIntParam("learnings", 100);
const ADR_LIMIT = positiveIntParam("adrs", 100);
const FRICTION_LIMIT = positiveIntParam("frictions", 100);

const AUDIT_STATUSES = ["success", "failure", "denied"];
const CANCELLABLE_RUN_STATES = new Set(["pending", "running"]);
const FRICTION_STATUSES = ["open", "triaged", "resolved"];

const $ = (id) => document.getElementById(id);

let searchQuery = "";
let activeStatuses = new Set(
  STATUS_ORDER.filter((s) => !DEFAULT_INACTIVE_STATUSES.has(s)),
);
let lastTasks = [];
let lastCrewPayload = { default_crew: null, crews: [] };
let lastRuns = [];
let lastDiagnostics = { metrics: [], errors: [], implement_one: [] };
let lastLearningPayload = { stats: {}, items: [] };
let lastAdrPayload = { stats: {}, items: [] };
let lastFrictionPayload = { stats: {}, tags: [], items: [] };
let activeTab = "tasks";
let activeDiagSubtab = "runs";
let activeKnowledgeSubtab = "learnings";
let runSort = { key: "when", dir: "desc" };
let expandedTaskIds = new Set();
let isRefreshing = false;
let taskActionNotice = null;
let crewUpdateErrors = new Map();
let activeLearningId = null;
let learningSearchQuery = "";
let activeAdrId = null;
let adrSearchQuery = "";
let activeFrictionId = null;
let frictionSearchQuery = "";

// Audit tab state
let lastAudit = [];
let auditFilter = {
  status: null,
  q: "",
  tool: null,
  role: null,
  // Filters audit Events by `execution_id` (the orbit invocation id). The CLI
  // SQLite audit table has no real `run_id` field, so this never identifies a
  // JobRun — see T20260427-26.
  execution_id: null,
  profile: null,
  // Time-window filter for the Events sub-tab. Accepts the same shorthands as
  // the API (`24h`, `7d`, `1w`, RFC3339); null means the API-side default.
  since: null,
  // Policy sub-tab only: scopes /api/diagnostics/denials to fs vs tool denials.
  policyKind: null,
};
let expandedAuditIds = new Set();
let activeAuditSubtab = "events";
let lastAuditPolicy = null;
let policySort = {
  by_profile: "count",
  by_target: "count",
  by_run: "count",
  by_execution: "count",
  by_agent: "count",
};

// Health strip state
let lastSummary = null;

// Run detail state
let activeRunId = null;
let activeRunDetail = null;
let activeRunEvents = [];
let activeRunLogs = [];
let activeRunSubtab = "steps";
let expandedStepIndices = new Set();

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

function requestJson(path, method, body) {
  const headers = { accept: "application/json" };
  const opts = {
    method,
    headers,
  };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  return fetch(path, opts).then(async (res) => {
    const text = await res.text();
    const body = text ? JSON.parse(text) : {};
    if (!res.ok) {
      throw new Error(body.error || `${path}: HTTP ${res.status}`);
    }
    return body;
  });
}

function postJson(path, body) {
  return requestJson(path, "POST", body);
}

function patchJson(path, body) {
  return requestJson(path, "PATCH", body);
}

function normalizeCrewPayload(payload) {
  const crews = Array.isArray(payload && payload.crews)
    ? payload.crews
      .filter((crew) => crew && crew.name)
      .map((crew) => ({
        name: String(crew.name),
        planner_model: crew.planner_model == null ? "" : String(crew.planner_model),
        implementer_model: crew.implementer_model == null ? "" : String(crew.implementer_model),
        reviewer_model: crew.reviewer_model == null ? "" : String(crew.reviewer_model),
        is_default: Boolean(crew.is_default),
      }))
    : [];
  crews.sort((a, b) => a.name.localeCompare(b.name));
  return {
    default_crew: payload && payload.default_crew ? String(payload.default_crew) : null,
    crews,
  };
}

function crewOptionsSignature() {
  return JSON.stringify(lastCrewPayload);
}

function explicitCrewValue(task) {
  return task && task.crew ? String(task.crew) : "";
}

function resolvedCrewName(task) {
  if (lastCrewPayload.default_crew) return lastCrewPayload.default_crew;
  if (!explicitCrewValue(task) && task && task.resolved_crew) {
    return String(task.resolved_crew);
  }
  return "workspace";
}

function crewOptionTitle(crew) {
  const parts = [
    `planner=${crew.planner_model || "-"}`,
    `implementer=${crew.implementer_model || "-"}`,
    `reviewer=${crew.reviewer_model || "-"}`,
  ];
  return parts.join(" · ");
}

function applyUpdatedTask(updatedTask) {
  const index = lastTasks.findIndex((task) => task.id === updatedTask.id);
  if (index >= 0) {
    lastTasks[index] = updatedTask;
  }
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
  ["job_run_id", "job_run"],
  ["created_at", "created"],
  ["updated_at", "updated"],
];

const RELATION_GROUPS = [
  ["blocked_by", "BlockedBy"],
  ["child_of", "ChildOf"],
  ["spawned_from", "SpawnedFrom"],
  ["regression_from", "RegressionFrom"],
  ["supersedes", "Supersedes"],
  ["related_to", "RelatedTo"],
];
const RELATION_GROUP_LABELS = new Map(RELATION_GROUPS);

function relationTypeKey(value) {
  if (!value) return "";
  return String(value)
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/-/g, "_")
    .toLowerCase();
}

function copyTaskIdWithNotice(taskId) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(taskId).catch(() => {});
  }
  taskActionNotice = `${taskId} is not in the filtered task list; copied ID`;
  renderTasks(lastTasks);
}

function findTaskRow(taskId) {
  return Array.from(document.querySelectorAll("#tasks-body .row"))
    .find((row) => row.dataset.key === `task-${taskId}`) || null;
}

function openVisibleTask(taskId) {
  const visible = filterTasks(lastTasks).some((task) => task.id === taskId);
  if (!visible) {
    copyTaskIdWithNotice(taskId);
    return;
  }
  expandedTaskIds.add(taskId);
  renderTasks(lastTasks);
  requestAnimationFrame(() => {
    const row = findTaskRow(taskId);
    if (!row) return;
    row.scrollIntoView({ behavior: "smooth", block: "center" });
    row.classList.add("data-changed");
    setTimeout(() => row.classList.remove("data-changed"), 1200);
  });
}

function buildTagRow(tags) {
  const wrap = el("div", { class: "detail-tag-row" });
  for (const tag of tags) {
    wrap.appendChild(el("span", { class: "chip", text: tag }));
  }
  return wrap;
}

function buildExternalRefs(refs) {
  const wrap = el("div");
  for (const ref of refs) {
    const label = `${ref.system || "external"}:${ref.id || ""}`;
    const line = el("div", { class: "external-ref-line" });
    if (ref.url) {
      const link = el("a", { text: label });
      link.href = ref.url;
      line.appendChild(link);
    } else {
      line.textContent = label;
    }
    wrap.appendChild(line);
  }
  return wrap;
}

function buildRelations(relations) {
  const byType = new Map(RELATION_GROUPS.map(([key]) => [key, []]));
  for (const relation of relations) {
    const key = relationTypeKey(relation.relation_type || relation.type);
    const target = relation.target == null ? "" : String(relation.target);
    if (!RELATION_GROUP_LABELS.has(key) || !target) continue;
    byType.get(key).push(target);
  }

  const wrap = el("div");
  for (const [key, label] of RELATION_GROUPS) {
    const targets = byType.get(key);
    if (!targets || targets.length === 0) continue;
    const group = el("div", { class: "relation-group" }, [
      el("span", { class: "label", text: label }),
    ]);
    for (const target of targets) {
      const btn = el("button", { class: "relation-target mono", text: target });
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        openVisibleTask(target);
      });
      group.appendChild(btn);
    }
    wrap.appendChild(group);
  }
  return wrap;
}

function reviewThreadStatus(thread) {
  const status = String(thread.status || "open").toLowerCase();
  return status === "resolved" ? "resolved" : "open";
}

function buildReviewThreads(threads) {
  const wrap = el("div", { class: "review-threads" });
  for (const thread of threads) {
    const messages = Array.isArray(thread.messages) ? thread.messages : [];
    const location = thread.path
      ? `${thread.path}${thread.line == null ? "" : `:${thread.line}`}`
      : "general";
    const block = el("div", { class: "review-thread" });
    block.appendChild(el("div", {
      class: "review-thread-header",
      text: `[${reviewThreadStatus(thread)}] ${location} · ${messages.length} messages`,
    }));
    for (const msg of messages) {
      const line = el("div", { class: "comment-line" }, [
        document.createTextNode(`[${fmtAbsTime(msg.at)}] `),
        el("span", { class: "author", text: msg.by || "?" }),
        document.createTextNode(`: ${msg.body || ""}`),
      ]);
      block.appendChild(line);
    }
    wrap.appendChild(block);
  }
  return wrap;
}

function fmtSize(bytes) {
  const value = Number(bytes);
  if (!Number.isFinite(value) || value < 0) return "0 bytes";
  if (value < 1024) return `${value} bytes`;
  const kb = value / 1024;
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
}

function artifactUrl(taskId, path) {
  const encodedPath = String(path)
    .split("/")
    .map((part) => encodeURIComponent(part))
    .join("/");
  return `/api/tasks/${encodeURIComponent(taskId)}/artifacts/${encodedPath}`;
}

function artifactMediaType(artifact, response) {
  return String(
    response.headers.get("content-type") || artifact.media_type || "application/octet-stream",
  ).split(";")[0].trim().toLowerCase();
}

function renderArtifactText(mediaType, text) {
  if (mediaType === "text/markdown" && typeof marked !== "undefined") {
    const view = el("div", { class: "markdown-body" });
    view.innerHTML = marked.parse(text);
    return view;
  }
  return el("pre", { text });
}

function buildArtifactPreview(artifact, response) {
  const mediaType = artifactMediaType(artifact, response);
  if (
    mediaType === "text/markdown" ||
    mediaType === "application/json" ||
    mediaType.endsWith("/yaml") ||
    mediaType.endsWith("+yaml") ||
    mediaType.startsWith("text/")
  ) {
    return response.text().then((text) => renderArtifactText(mediaType, text));
  }
  return response.blob().then((blob) => {
    const link = el("a", { text: `Download ${artifact.path}` });
    link.href = URL.createObjectURL(blob);
    link.download = String(artifact.path).split("/").pop() || artifact.path;
    return link;
  });
}

function buildArtifacts(task) {
  const wrap = el("div", { class: "artifacts" });
  for (const artifact of task.artifacts) {
    const path = String(artifact.path || "");
    const mediaType = String(artifact.media_type || "application/octet-stream");
    const row = el("div", {
      class: "artifact-row",
      text: `${path} · ${mediaType} · ${fmtSize(artifact.size_bytes)}`,
    });
    const preview = el("div", { class: "artifact-preview" });
    preview.hidden = true;
    row.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (preview.dataset.loaded === "true" && !preview.hidden) {
        preview.hidden = true;
        return;
      }
      if (preview.dataset.loaded === "true") {
        preview.hidden = false;
        return;
      }
      preview.hidden = false;
      preview.textContent = "loading...";
      try {
        const response = await fetch(artifactUrl(task.id, path));
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        preview.replaceChildren(await buildArtifactPreview(artifact, response));
        preview.dataset.loaded = "true";
      } catch (error) {
        preview.textContent = `Unable to load ${path}: ${error.message}`;
      }
    });
    wrap.appendChild(row);
    wrap.appendChild(preview);
  }
  return wrap;
}

function buildTaskDetail(task) {
  const detail = el("div", { class: "row-detail split-layout" });
  detail.addEventListener("click", (e) => e.stopPropagation());

  const leftCol = el("div", { class: "detail-main" });
  const rightCol = el("div", { class: "detail-side" });

  const addField = (parent, title, child, collapsible = false, collapsed = false) => {
    let classes = "field-block";
    if (collapsible) classes += " collapsible";
    if (collapsed) classes += " collapsed";
    const block = el("div", { class: classes });
    const h4 = el("h4", { text: title });
    if (collapsible) {
      h4.addEventListener("click", (e) => {
        e.stopPropagation();
        block.classList.toggle("collapsed");
      });
    }
    block.appendChild(h4);
    block.appendChild(child);
    parent.appendChild(block);
  };

  if (task.description && task.description.trim()) {
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.description);
    } else {
      view.textContent = task.description;
    }
    addField(leftCol, "description", view);
  }

  if (Array.isArray(task.acceptance_criteria) && task.acceptance_criteria.length > 0) {
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
    addField(leftCol, "acceptance criteria", ul, true, true);
  }

  if (task.plan && task.plan.trim()) {
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.plan);
    } else {
      view.textContent = task.plan;
    }
    addField(leftCol, "plan", view, true, true);
  }

  if (task.execution_summary && task.execution_summary.trim()) {
    const view = el("div", { class: "markdown-body" });
    if (typeof marked !== "undefined") {
      view.innerHTML = marked.parse(task.execution_summary);
    } else {
      view.textContent = task.execution_summary;
    }
    addField(leftCol, "execution summary", view, true, true);
  }

  if (Array.isArray(task.review_threads) && task.review_threads.length > 0) {
    addField(leftCol, "review threads", buildReviewThreads(task.review_threads), true, true);
  }

  if (Array.isArray(task.artifacts) && task.artifacts.length > 0) {
    addField(leftCol, "artifacts", buildArtifacts(task), true, true);
  }

  if (Array.isArray(task.tags) && task.tags.length > 0) {
    rightCol.appendChild(buildTagRow(task.tags));
  }

  const meta = el("div", { class: "meta-list" });
  let metaCount = 0;
  for (const [key, label] of TASK_META_FIELDS) {
    const v = task[key];
    if (v == null || v === "") continue;
    const display = key.endsWith("_at") ? fmtAbsTime(v) : String(v);
    const value = el("span", { class: "value" });
    if (key === "job_run_id") {
      const link = el("a", { text: display });
      link.href = `#runs?run_id=${encodeURIComponent(display)}`;
      value.appendChild(link);
    } else {
      value.textContent = display;
    }
    const span = el("div", { class: "meta-item" }, [
      el("span", { class: "label", text: `${label}` }),
      value,
    ]);
    meta.appendChild(span);
    metaCount++;
  }
  if (metaCount > 0) addField(rightCol, "details", meta);

  if (Array.isArray(task.external_refs) && task.external_refs.length > 0) {
    addField(rightCol, "external refs", buildExternalRefs(task.external_refs));
  }

  if (Array.isArray(task.relations) && task.relations.length > 0) {
    const relations = buildRelations(task.relations);
    if (relations.children.length > 0) addField(rightCol, "relations", relations);
  }

  if (Array.isArray(task.context_files) && task.context_files.length > 0) {
    const ul = el("ul", { class: "file-list" });
    for (const path of task.context_files) {
      ul.appendChild(el("li", { text: path }));
    }
    addField(rightCol, "context files", ul);
  }

  if (Array.isArray(task.history) && task.history.length > 0) {
    const wrap = el("div");
    const recent = task.history.slice(-5).reverse();
    for (const h of recent) {
      const note = h.note ? ` (${h.note})` : "";
      const line = el("div", { class: "history-line" }, [
        document.createTextNode(`[${fmtAbsTime(h.at)}] `),
        el("span", { class: "actor", text: h.by || "?" }),
        document.createTextNode(`: ${h.event}${note}`),
      ]);
      wrap.appendChild(line);
    }
    addField(rightCol, "recent history", wrap);
  }

  if (Array.isArray(task.comments) && task.comments.length > 0) {
    const wrap = el("div");
    for (const c of task.comments) {
      const line = el("div", { class: "comment-line" }, [
        document.createTextNode(`[${fmtAbsTime(c.at)}] `),
        el("span", { class: "author", text: c.by || "?" }),
        document.createTextNode(`: ${c.message || ""}`),
      ]);
      wrap.appendChild(line);
    }
    addField(rightCol, "comments", wrap);
  }

  detail.appendChild(leftCol);
  detail.appendChild(rightCol);
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
  const statusSelect = buildStatusUpdateControl(task, detail);
  if (statusSelect) actions.appendChild(statusSelect);
  return actions;
}

function buildStatusUpdateControl(task, detail) {
  const targets = STATUS_UPDATE_TARGETS.filter((status) => status !== task.status);
  if (targets.length === 0) return null;

  const select = el("select", {
    class: "action status-update",
    title: `Update status for ${task.id}`,
  });
  const placeholder = el("option", { text: "set status" });
  placeholder.value = "";
  placeholder.disabled = true;
  placeholder.selected = true;
  select.appendChild(placeholder);

  for (const status of targets) {
    const option = el("option", { text: status });
    option.value = status;
    select.appendChild(option);
  }

  select.addEventListener("click", (e) => e.stopPropagation());
  select.addEventListener("change", (e) => {
    e.stopPropagation();
    const targetStatus = select.value;
    if (!targetStatus) return;
    runAction(
      task,
      "status",
      detail,
      { status: targetStatus },
      select,
      {
        method: "PATCH",
        path: `/api/tasks/${encodeURIComponent(task.id)}`,
        collapseOnSuccess: targetStatus === "done",
        successNotice: targetStatus === "done"
          ? `Task ${task.id} marked done; it is no longer shown in the default dashboard list.`
          : null,
        onFailure: () => {
          select.value = "";
        },
      },
    );
  });
  return select;
}

function buildCrewUpdateControl(task) {
  const cell = el("span", { class: "crew-cell" });
  const select = el("select", {
    class: "task-crew-select mono",
    title: `Update crew for ${task.id}`,
  });
  const currentValue = explicitCrewValue(task);
  select.dataset.currentValue = currentValue;

  const defaultOption = el("option", {
    text: `default: ${resolvedCrewName(task)}`,
  });
  defaultOption.value = "";
  select.appendChild(defaultOption);

  const crews = Array.isArray(lastCrewPayload.crews) ? lastCrewPayload.crews : [];
  for (const crew of crews) {
    const option = el("option", {
      text: crew.name,
      title: crewOptionTitle(crew),
    });
    option.value = crew.name;
    select.appendChild(option);
  }

  if (currentValue && !crews.some((crew) => crew.name === currentValue)) {
    const option = el("option", {
      text: currentValue,
      title: "Configured crew no longer found",
    });
    option.value = currentValue;
    select.appendChild(option);
  }

  if (crews.length === 0) {
    select.disabled = true;
    defaultOption.textContent = "crew unavailable";
  }

  select.value = currentValue;
  for (const eventName of ["pointerdown", "mousedown", "click", "keydown"]) {
    select.addEventListener(eventName, (event) => event.stopPropagation());
  }
  select.addEventListener("change", (event) => {
    event.stopPropagation();
    updateTaskCrew(task, select);
  });

  cell.appendChild(select);
  const error = crewUpdateErrors.get(task.id);
  if (error) {
    cell.appendChild(el("span", { class: "crew-error", text: error }));
  }
  return cell;
}

async function updateTaskCrew(task, select) {
  const previousValue = select.dataset.currentValue || "";
  const nextValue = select.value || "";
  if (nextValue === previousValue) return;

  crewUpdateErrors.delete(task.id);
  select.disabled = true;
  try {
    const updatedTask = await patchJson(`/api/tasks/${encodeURIComponent(task.id)}`, {
      crew: nextValue || null,
    });
    applyUpdatedTask(updatedTask);
    crewUpdateErrors.delete(task.id);
    renderTasks(lastTasks);
  } catch (error) {
    select.value = previousValue;
    select.dataset.currentValue = previousValue;
    select.disabled = false;
    crewUpdateErrors.set(
      task.id,
      `crew update failed: ${error.message || String(error)}`,
    );
    renderTasks(lastTasks);
    console.error(error);
  }
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

async function runAction(task, kind, detail, body, btnNode, opts = {}) {
  // Disable action controls while in flight to prevent double-clicks.
  for (const b of detail.querySelectorAll(".action")) b.disabled = true;
  let oldText = "";
  if (btnNode && btnNode.tagName === "BUTTON") {
    oldText = btnNode.textContent;
    btnNode.innerHTML = `<span class="spinner"></span>wait`;
  }
  // Clear any prior error
  const prior = detail.querySelector(".action-error");
  if (prior) prior.remove();
  try {
    const res = await fetch(opts.path || `/api/tasks/${encodeURIComponent(task.id)}/${kind}`, {
      method: opts.method || "POST",
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
    if (opts.collapseOnSuccess !== false) expandedTaskIds.delete(task.id);
    if (opts.successNotice) taskActionNotice = opts.successNotice;
    await refreshDashboard();
  } catch (err) {
    for (const b of detail.querySelectorAll(".action")) b.disabled = false;
    if (btnNode && btnNode.tagName === "BUTTON") btnNode.textContent = oldText;
    if (opts.onFailure) opts.onFailure();
    const errEl = el("div", { class: "action-error", text: String(err.message || err) });
    detail.prepend(errEl);
  }
}

function takeTaskActionNotice() {
  if (!taskActionNotice) return null;
  const notice = el("div", { class: "task-action-notice", text: taskActionNotice });
  notice.dataset.key = "task-action-notice";
  notice.dataset.hash = taskActionNotice;
  taskActionNotice = null;
  return notice;
}

function renderTasks(tasks) {
  const body = $("tasks-body");
  const frag = document.createDocumentFragment();
  const notice = takeTaskActionNotice();
  
  const filtered = filterTasks(tasks);
  $("tasks-count").textContent =
    filtered.length === tasks.length
      ? `${tasks.length}`
      : `${filtered.length}/${tasks.length}`;
  if (filtered.length === 0) {
    const defaultText = tasks.length === 0 ? "No tasks available." : "No tasks match filter.";
    const emptyState = el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: defaultText })
    ]);
    syncNodes(body, notice ? [notice, emptyState] : [emptyState]);
    return;
  }
  const groups = new Map();
  if (notice) frag.appendChild(notice);
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
        buildCrewUpdateControl(t),
      ]);
      row.dataset.key = `task-${t.id}`;
      // Basic hash based on row presentation parameters + expanded state
      row.dataset.hash = `${t.id}-${t.title}-${t.priority}-${t.type}-${t.crew || ""}-${t.resolved_crew || ""}-${crewOptionsSignature()}-${crewUpdateErrors.get(t.id) || ""}-${expandedTaskIds.has(t.id)}`;
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
      openVisibleTask(taskId);
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

  const canonicalRows = canonicalScoreboardRows(agentsMap);
  const otherRows = entries
    .filter(([name]) => !CANONICAL_SCOREBOARD_SET.has(name))
    .filter(([, agent]) => scoreboardSignalForColumns(agent, OTHER_SCOREBOARD_COLUMNS) > 0)
    .sort(([a], [b]) => a.localeCompare(b));

  const sections = [
    renderScoreboardSection("Delivery", DELIVERY_SCOREBOARD_COLUMNS, canonicalRows),
    renderScoreboardSection("Review", REVIEW_SCOREBOARD_COLUMNS, canonicalRows),
    renderScoreboardSection("Knowledge", KNOWLEDGE_SCOREBOARD_COLUMNS, canonicalRows),
    renderScoreboardSection("Operations", OPERATIONS_SCOREBOARD_COLUMNS, canonicalRows),
    renderScoreboardSection("Planning Duels", PLANNING_SCOREBOARD_COLUMNS, canonicalRows),
    renderDuelMatrixSection(summary),
    renderScoreboardSection("Attribution Cleanup", OTHER_SCOREBOARD_COLUMNS, otherRows),
  ];

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

function renderScoreboardSection(title, columns, rows) {
  const section = el("section", { class: "scoreboard-section" });
  section.appendChild(
    el("div", { class: "scoreboard-section-header" }, [
      el("span", { class: "scoreboard-section-title", text: title }),
      el("span", { class: "scoreboard-section-count", text: String(rows.length) }),
    ]),
  );
  section.appendChild(renderScoreboardTable(columns, rows, title));
  return section;
}

function renderScoreboardTable(columns, rows, sectionTitle) {
  if (!rows.length) {
    return el("div", { class: "empty-state compact", text: "No rows." });
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
  for (const [name, agent] of rows) {
    const tr = el("tr");
    for (const col of columns) {
      const td = renderScoreboardCell(name, agent, col);
      tr.appendChild(td);
    }
    tr.dataset.key = `scoreboard-${sectionTitle}-${name}`;
    tr.dataset.hash = JSON.stringify(agent);
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  return table;
}

function renderScoreboardCell(name, agent, col) {
  let cellText;
  let extra = "";
  let titleText = col.title;

  if (col.key === "agent") {
    cellText = name;
    titleText = `${name} — click to filter audit by role`;
    extra = " clickable";
  } else if (col.format === "pair") {
    const pair = formatScoreboardPair(agent, col);
    cellText = pair.text;
    titleText = pair.title;
    if (pair.zero) extra = " zero";
  } else {
    const value = col.compute ? col.compute(agent) : readPath(agent, col.key);
    const num = asScoreboardNumber(value);
    cellText = fmtScoreboardCount(num);
    if (num === 0) extra = " zero";
  }

  const td = el("td", {
    class: (col.num ? "num" : col.key === "agent" ? "agent" : "") + extra,
    text: cellText,
    title: titleText,
  });
  if (col.key === "agent") {
    td.addEventListener("click", () => navigateToRole(name));
  }
  return td;
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
        text: `${fmtScoreboardCount(wins)}/${fmtScoreboardCount(losses)}`,
        title: `${family} vs ${opponent}: ${wins} wins / ${losses} losses (${runs} runs)`,
      }));
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
  const open = () => openVisibleTask(taskId);
  if (lastTasks.length > 0 && lastCrewPayload.crews.length > 0) {
    open();
    return;
  }
  fetchAndRenderTasks().then(() => {
    open();
  }).catch(() => copyTaskIdWithNotice(taskId));
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
const AUDIT_SUBTABS = ["events", "policy"];
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
    auditFilter.status = query.get("status") || null;
    auditFilter.tool = query.get("tool") || null;
    auditFilter.role = query.get("role") || null;
    // Accept legacy `run_id=` URLs as an alias of `execution_id=`. The CLI
    // audit table never had a real run_id; both names point at execution_id.
    auditFilter.execution_id =
      query.get("execution_id") || query.get("run_id") || null;
    auditFilter.profile = query.get("profile") || null;
    auditFilter.q = query.get("q") || "";
    auditFilter.since = query.get("since") || null;
    const kindParam = query.get("kind");
    auditFilter.policyKind = kindParam === "fs" || kindParam === "tool" ? kindParam : null;
    const sub = AUDIT_SUBTABS.includes(segments[1]) ? segments[1] : activeAuditSubtab;
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

function buildAuditHash() {
  const sp = new URLSearchParams();
  // The Audit Events tab and the Policy sub-tab serialize different filter
  // axes. The shared `auditFilter` object holds all of them; here we project
  // only the fields meaningful for the active sub-tab so the hash stays
  // self-describing and reload-safe.
  if (activeAuditSubtab === "policy") {
    if (auditFilter.policyKind) sp.set("kind", auditFilter.policyKind);
    if (auditFilter.profile) sp.set("profile", auditFilter.profile);
    if (auditFilter.role) sp.set("role", auditFilter.role);
  } else {
    if (auditFilter.since) sp.set("since", auditFilter.since);
    if (auditFilter.status) sp.set("status", auditFilter.status);
    if (auditFilter.tool) sp.set("tool", auditFilter.tool);
    if (auditFilter.role) sp.set("role", auditFilter.role);
    if (auditFilter.execution_id) sp.set("execution_id", auditFilter.execution_id);
    if (auditFilter.profile) sp.set("profile", auditFilter.profile);
    if (auditFilter.q) sp.set("q", auditFilter.q);
  }
  const path = activeAuditSubtab && activeAuditSubtab !== "events"
    ? `audit/${activeAuditSubtab}`
    : "audit";
  const qs = sp.toString();
  return qs ? `#${path}?${qs}` : `#${path}`;
}

function setAuditSubtab(name) {
  if (!AUDIT_SUBTABS.includes(name)) name = "events";
  activeAuditSubtab = name;
  for (const btn of document.querySelectorAll("#audit-subtabs .subtab")) {
    btn.classList.toggle("active", btn.dataset.subtab === name);
  }
  const eventsCtl = $("audit-events-controls");
  const eventsBody = $("audit-body");
  const policyBody = $("audit-policy-body");
  if (eventsCtl) eventsCtl.style.display = name === "events" ? "" : "none";
  if (eventsBody) eventsBody.style.display = name === "events" ? "" : "none";
  if (policyBody) policyBody.style.display = name === "policy" ? "" : "none";
  const title = $("audit-title");
  if (title) title.textContent = name === "policy" ? "Policy Denials" : "Audit Events";
}

function syncAuditControls() {
  const search = $("audit-search");
  if (search && search.value !== (auditFilter.q || "")) {
    search.value = auditFilter.q || "";
  }
  for (const chip of document.querySelectorAll("#audit-filter .chip")) {
    const status = chip.dataset.status;
    chip.classList.toggle("active", auditFilter.status === status);
  }
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
    renderDiagnostics();
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
      activeAuditSubtab = btn.dataset.subtab;
      setAuditSubtab(activeAuditSubtab);
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

const DIAG_ERRORS_COLUMNS = [
  { key: "ts", label: "time", num: false, render: (v) => fmtRelative(v) },
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
      return truncate(full, 220);
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
    if (row.job_run) {
      tr.classList.add("clickable");
      tr.title = "Open owning run";
      tr.addEventListener("click", () => {
        const stepQuery = row.step_index == null ? "" : `?step=${encodeURIComponent(row.step_index)}`;
        setActiveTab(`runs/${encodeURIComponent(row.job_run)}${stepQuery}`);
      });
    }
    frag.appendChild(tr);
  }
  
  syncNodes(tbody, Array.from(frag.children));
}

function renderDiagnostics() {
  const sub = activeDiagSubtab;
  const rows = lastDiagnostics[sub] || [];
  $("diag-count").textContent = `${rows.length}`;
  const columns =
    sub === "metrics"
      ? DIAG_METRICS_COLUMNS
      : DIAG_ERRORS_COLUMNS;
  renderDiagnosticsTable(
    rows,
    columns,
  );

  const sidePanel = $("diagnostics-side-panel");
  if (sidePanel) {
    renderImplementOneCard($("diag-implement-one-body"), lastDiagnostics.implement_one || []);
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

function renderImplementOneCard(container, rows) {
  container.innerHTML = "";
  if (rows.length === 0) {
    container.appendChild(el("div", { class: "empty", text: "No implement_one runs in last 30d." }));
    return;
  }

  const durCols = [
    { key: "actor", label: "actor" },
    { key: "n", label: "n", num: true },
    { key: "avg", label: "avg", num: true, format: fmtDuration },
    { key: "p50", label: "p50", num: true, format: fmtDuration },
    { key: "p95", label: "p95", num: true, format: fmtDuration }
  ];
  renderMetricsCard(container, "Average implement_one duration by actor (30d)", rows, durCols);
}

function fetchAndCacheCrews() {
  return fetchJson("/api/crews").then((payload) => {
    lastCrewPayload = normalizeCrewPayload(payload);
    return lastCrewPayload;
  });
}

function fetchAndRenderTasks() {
  return Promise.all([
    fetchJson("/api/tasks"),
    fetchAndCacheCrews(),
  ]).then(([tasks]) => {
    lastTasks = tasks;
    renderTasks(tasks);
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
    if (activeAuditSubtab === "policy") {
      jobs.push(fetchAndRenderPolicy());
    } else {
      jobs.push(fetchAndRenderAudit());
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
          renderDiagnostics();
        })
      );
    } else if (activeDiagSubtab === "errors") {
      jobs.push(
        fetchJson(`/api/diagnostics/errors?limit=${DIAG_LIMIT}`).then((rows) => {
          lastDiagnostics.errors = rows;
          renderDiagnostics();
        })
      );
    }

    jobs.push(
      fetchJson(`/api/diagnostics/implement_one`)
        .then((implOne) => {
          lastDiagnostics.implement_one = implOne.implement_one_by_actor || [];
          const sidePanel = $("diagnostics-side-panel");
          if (sidePanel) {
            renderImplementOneCard($("diag-implement-one-body"), lastDiagnostics.implement_one);
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

function fetchAndRenderAudit() {
  const sp = new URLSearchParams();
  sp.set("limit", String(AUDIT_LIMIT));
  if (auditFilter.since) sp.set("since", auditFilter.since);
  if (auditFilter.status) sp.set("status", auditFilter.status);
  if (auditFilter.tool) sp.set("tool", auditFilter.tool);
  if (auditFilter.role) sp.set("role", auditFilter.role);
  if (auditFilter.execution_id) sp.set("execution_id", auditFilter.execution_id);
  if (auditFilter.profile) sp.set("profile", auditFilter.profile);
  if (auditFilter.q) sp.set("q", auditFilter.q);
  return fetchJson(`/api/audit?${sp.toString()}`).then((events) => {
    lastAudit = events;
    renderAudit(events);
  });
}

function fetchAndRenderSummary() {
  return fetchJson(`/api/audit/summary?since=24h`).then((data) => {
    lastSummary = data;
    renderHealthStrip(data);
    renderAuditSummary(data);
  });
}

function renderAuditSummary(data) {
  const container = $("audit-summary-body");
  if (!container) return;
  container.innerHTML = "";

  const createCard = (title, renderBody) => {
    const card = el("div", { class: "audit-summary-card" });
    card.appendChild(el("div", { class: "card-title", text: title }));
    const body = el("div", { class: "card-body" });
    renderBody(body);
    card.appendChild(body);
    return card;
  };

  const renderTable = (items, cols, onRowClick) => {
    return (body) => {
      if (!items || items.length === 0) {
        body.appendChild(el("div", { class: "empty", text: "No data" }));
        return;
      }
      const table = el("table", { class: "summary-table" });
      const thead = el("thead");
      const tr = el("tr");
      for (const c of cols) tr.appendChild(el("th", { class: c.num ? "num" : "", text: c.label }));
      thead.appendChild(tr);
      table.appendChild(thead);

      const tbody = el("tbody");
      for (const item of items) {
        const row = el("tr");
        if (onRowClick) {
          row.classList.add("clickable");
          row.addEventListener("click", () => onRowClick(item));
        }
        for (const c of cols) {
          const val = c.format ? c.format(item[c.key]) : item[c.key];
          row.appendChild(el("td", { class: c.num ? "num" : "", text: val }));
        }
        tbody.appendChild(row);
      }
      table.appendChild(tbody);
      body.appendChild(table);
    };
  };

  const filterByTool = (item) => {
    auditFilter.tool = auditFilter.tool === item.tool ? null : item.tool;
    syncAuditControls();
    window.location.hash = buildAuditHash();
  };

  if (data.failures_by_tool) {
    container.appendChild(createCard("Failures by tool", renderTable(
      data.failures_by_tool,
      [{ key: "tool", label: "tool" }, { key: "count", label: "fails", num: true }, { key: "mcp", label: "mcp", num: true }, { key: "cli", label: "cli", num: true }],
      filterByTool
    )));
  }

  if (data.duration_by_tool) {
    container.appendChild(createCard("Top duration (avg)", renderTable(
      data.duration_by_tool,
      [
        { key: "tool", label: "tool" },
        { key: "count", label: "count", num: true },
        { key: "avg", label: "avg", num: true, format: (v) => fmtDuration(v) },
        { key: "p95", label: "p95", num: true, format: (v) => fmtDuration(v) }
      ],
      filterByTool
    )));
  }

  if (data.failure_rate_by_tool) {
    container.appendChild(createCard("Failure rate %", renderTable(
      data.failure_rate_by_tool,
      [
        { key: "tool", label: "tool" },
        { key: "rate", label: "rate", num: true, format: (r) => (r * 100).toFixed(1) + "%" },
        { key: "mcp_rate", label: "mcp", num: true, format: (r) => (r * 100).toFixed(1) + "%" },
        { key: "cli_rate", label: "cli", num: true, format: (r) => (r * 100).toFixed(1) + "%" }
      ],
      filterByTool
    )));
  }

  if (data.denials_by_tool || data.denials_by_reason) {
    const card = el("div", { class: "audit-summary-card" });
    card.appendChild(el("div", { class: "card-title", text: "Denials" }));
    const body = el("div", { class: "card-body" });
    const sectionLabel = (txt) => {
      const lbl = el("div", { class: "card-subtitle", text: txt });
      lbl.style.cssText = "padding: 6px 12px; font-size: 10px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--fg-dim); border-bottom: 1px solid var(--border); background: rgba(255,255,255,0.02);";
      return lbl;
    };
    const toolRows = data.denials_by_tool || [];
    body.appendChild(sectionLabel("By tool"));
    renderTable(
      toolRows,
      [{ key: "tool", label: "tool" }, { key: "count", label: "count", num: true }],
      filterByTool
    )(body);
    const reasonRows = data.denials_by_reason || [];
    body.appendChild(sectionLabel("By reason"));
    renderTable(
      reasonRows,
      [{ key: "reason", label: "reason" }, { key: "count", label: "count", num: true }],
      null
    )(body);
    card.appendChild(body);
    container.appendChild(card);
  }

  if (data.role_split) {
    container.appendChild(createCard("Role split", renderTable(
      data.role_split,
      [{ key: "label", label: "role" }, { key: "count", label: "count", num: true }, { key: "mcp", label: "mcp", num: true }, { key: "cli", label: "cli", num: true }],
      (item) => {
        auditFilter.role = auditFilter.role === item.label ? null : item.label;
        syncAuditControls();
        window.location.hash = buildAuditHash();
      }
    )));
  }

  if (data.mcp_vs_cli_split) {
    container.appendChild(createCard("MCP vs CLI", renderTable(
      data.mcp_vs_cli_split,
      [{ key: "label", label: "surface" }, { key: "count", label: "count", num: true }],
      null
    )));
  }
}

function fetchAndRenderPolicy() {
  const sp = new URLSearchParams();
  sp.set("since", "24h");
  if (auditFilter.policyKind) sp.set("kind", auditFilter.policyKind);
  if (auditFilter.profile) sp.set("profile", auditFilter.profile);
  if (auditFilter.role) sp.set("agent", auditFilter.role);
  return fetchJson(`/api/diagnostics/denials?${sp.toString()}`).then((data) => {
    lastAuditPolicy = data;
    renderPolicy(data);
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

const POLICY_TABLES = [
  {
    id: "by_profile",
    label: "By Profile",
    nameField: "name",
    header: "profile",
    filterKey: "profile",
  },
  {
    id: "by_target",
    label: "By Target",
    nameField: "name",
    header: "target",
    filterKey: null,
  },
  {
    id: "by_run",
    label: "By JobRun",
    nameField: "run_id",
    header: "job_run_id",
    navigateTo: "job_run",
  },
  {
    id: "by_execution",
    label: "By Audit Invocation",
    nameField: "execution_id",
    header: "execution_id",
    navigateTo: "audit_execution",
  },
  {
    id: "by_agent",
    label: "By Agent",
    nameField: "agent",
    header: "agent",
    filterKey: "role",
  },
];

function renderPolicy(data) {
  const body = $("audit-policy-body");
  if (!body) return;
  $("audit-count").textContent = `${data && data.total ? data.total : 0}`;

  if (!data || (data.total || 0) === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No denials in the last 24h." }),
    ])]);
    return;
  }

  const sections = [];
  const recent = buildRecentDenials(data.recent_denials || []);
  const causes = buildTopCauses(data.top_causes || []);
  if (recent) sections.push(recent);
  if (causes) sections.push(causes);

  const grid = el("div", { class: "policy-grid" });
  for (const tbl of POLICY_TABLES) {
    const cell = el("div", { class: "policy-cell" });
    cell.appendChild(el("h5", { text: tbl.label }));
    const rawRows = (data[tbl.id] || []).slice();
    const sortMode = policySort[tbl.id] || "count";
    rawRows.sort((a, b) => {
      if (sortMode === "name") {
        return String(a[tbl.nameField] || "").localeCompare(String(b[tbl.nameField] || ""));
      }
      return (b.count || 0) - (a.count || 0);
    });
    cell.appendChild(buildPolicyTable(tbl, rawRows, sortMode));
    grid.appendChild(cell);
  }
  sections.push(grid);
  syncNodes(body, sections);
}

function buildPolicyTable(spec, rows, sortMode) {
  const table = el("table", { class: "policy-table" });
  const thead = el("thead");
  const headRow = el("tr");
  const nameTh = el("th", { text: spec.header || spec.nameField });
  if (sortMode === "name") {
    const arrow = el("span", { class: "sort-arrow", text: "▼" });
    nameTh.appendChild(arrow);
  }
  nameTh.addEventListener("click", () => {
    policySort[spec.id] = "name";
    if (lastAuditPolicy) renderPolicy(lastAuditPolicy);
  });
  headRow.appendChild(nameTh);
  const countTh = el("th", { class: "num", text: "count" });
  if (sortMode === "count") {
    const arrow = el("span", { class: "sort-arrow", text: "▼" });
    countTh.appendChild(arrow);
  }
  countTh.addEventListener("click", () => {
    policySort[spec.id] = "count";
    if (lastAuditPolicy) renderPolicy(lastAuditPolicy);
  });
  headRow.appendChild(countTh);
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = el("tbody");
  if (rows.length === 0) {
    const tr = el("tr");
    const td = el("td", { class: "value-name", text: "—" });
    td.colSpan = 2;
    tr.appendChild(td);
    tbody.appendChild(tr);
  } else {
    for (const row of rows) {
      const name = String(row[spec.nameField] ?? "");
      const tr = el("tr", { title: name });
      tr.appendChild(el("td", { class: "value-name", text: name }));
      tr.appendChild(el("td", { class: "num", text: String(row.count || 0) }));
      if (spec.navigateTo === "job_run") {
        tr.classList.add("clickable");
        tr.addEventListener("click", () => navigateToRun(name));
      } else if (spec.navigateTo === "audit_execution") {
        tr.classList.add("clickable");
        tr.addEventListener("click", () => navigateToAuditExecution(name));
      } else if (spec.filterKey) {
        tr.classList.add("clickable");
        tr.addEventListener("click", () => {
          auditFilter[spec.filterKey] = name;
          activeAuditSubtab = "events";
          window.location.hash = buildAuditHash();
        });
      }
      tbody.appendChild(tr);
    }
  }
  table.appendChild(tbody);
  return table;
}

function buildTopCauses(rows) {
  if (!rows.length) return null;
  const section = el("div", { class: "policy-section" });
  section.appendChild(el("h5", { text: "Top Causes" }));
  const table = el("table", { class: "policy-table policy-cause-table" });
  const thead = el("thead");
  const headRow = el("tr");
  for (const label of ["cause", "target", "count", "latest"]) {
    headRow.appendChild(el("th", { class: label === "count" ? "num" : "", text: label }));
  }
  thead.appendChild(headRow);
  table.appendChild(thead);
  const tbody = el("tbody");
  for (const row of rows) {
    const tr = el("tr");
    tr.appendChild(el("td", {
      class: "value-name",
      text: row.cause || "-",
      title: row.cause || "",
    }));
    tr.appendChild(el("td", {
      class: "value-name muted",
      text: row.target || "-",
      title: row.target || "",
    }));
    tr.appendChild(el("td", { class: "num", text: String(row.count || 0) }));
    tr.appendChild(el("td", {
      class: "muted mono",
      text: row.latest_ts ? fmtRelative(row.latest_ts) : "-",
    }));
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  section.appendChild(table);
  return section;
}

function buildRecentDenials(rows) {
  if (!rows.length) return null;
  const section = el("div", { class: "policy-section" });
  section.appendChild(el("h5", { text: "Recent Denials" }));
  const table = el("table", { class: "policy-table policy-recent-table" });
  const thead = el("thead");
  const headRow = el("tr");
  for (const label of ["time", "target", "cause", "identity", "details"]) {
    headRow.appendChild(el("th", { text: label }));
  }
  thead.appendChild(headRow);
  table.appendChild(thead);
  const tbody = el("tbody");
  for (const row of rows) {
    const tr = el("tr");
    tr.appendChild(el("td", {
      class: "muted mono",
      text: row.timestamp ? fmtRelative(row.timestamp) : "-",
    }));
    tr.appendChild(el("td", {
      class: "value-name",
      text: row.target || "-",
      title: row.target || "",
    }));
    tr.appendChild(el("td", {
      class: "value-name",
      text: row.cause || row.denial_kind || "-",
      title: row.cause || "",
    }));
    const identity = el("td");
    identity.appendChild(buildPolicyIdentityAction(row));
    tr.appendChild(identity);
    const details = policyDetailText(row);
    tr.appendChild(el("td", { class: "policy-detail", text: details, title: details }));
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  section.appendChild(table);
  return section;
}

function buildPolicyIdentityAction(row) {
  const identityId = row.identity_id || row.job_run_id || row.execution_id || "";
  if (!identityId) return el("span", { class: "muted", text: "-" });
  const isJobRun = row.identity_type === "job_run" && row.job_run_id;
  const label = isJobRun ? "JobRun" : "Audit";
  const btn = el("button", {
    class: "policy-link",
    text: `${label} ${truncate(identityId, 18)}`,
    title: identityId,
  });
  btn.addEventListener("click", (event) => {
    event.stopPropagation();
    if (isJobRun) navigateToRun(identityId);
    else navigateToAuditExecution(identityId);
  });
  return btn;
}

function policyDetailText(row) {
  const parts = [];
  if (row.actor) parts.push(`actor ${row.actor}`);
  const taskIds = row.requested_task_ids || [];
  if (taskIds.length) parts.push(`tasks ${taskIds.join(", ")}`);
  const files = row.requested_files || [];
  if (files.length) {
    const suffix = files.length > 2 ? " +" + (files.length - 2) : "";
    parts.push(`files ${files.slice(0, 2).join(", ")}${suffix}`);
  }
  const conflicts = row.conflicts || [];
  if (conflicts.length) {
    const first = conflicts[0] || {};
    const holder = [first.held_by, first.held_by_id].filter(Boolean).join(" ");
    parts.push(holder ? `held by ${holder}` : `${conflicts.length} conflicts`);
  }
  return parts.join(" · ") || row.denial_kind || "-";
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

function navigateToAuditExecution(executionId) {
  auditFilter = {
    status: null,
    q: "",
    tool: null,
    role: null,
    execution_id: executionId,
    profile: null,
    since: null,
    policyKind: null,
  };
  activeAuditSubtab = "events";
  syncAuditControls();
  window.location.hash = buildAuditHash();
}

/// Navigates to the Audit tab pre-filtered by `role` (audit `role` ≈ scoreboard
/// agent name). Clears unrelated filters so the landing page is the role view.
function navigateToRole(role) {
  auditFilter = {
    status: null,
    q: "",
    tool: null,
    role,
    execution_id: null,
    profile: null,
    since: null,
    policyKind: null,
  };
  activeAuditSubtab = "events";
  syncAuditControls();
  window.location.hash = buildAuditHash();
}

function buildAuditChips() {
  const container = $("audit-filter");
  if (!container) return;
  container.innerHTML = "";
  const allChip = el("button", { class: "chip", text: "all" });
  allChip.addEventListener("click", () => {
    auditFilter.status = null;
    syncAuditControls();
    setActiveTab("audit" + buildAuditHash().slice(6), { refresh: true });
  });
  container.appendChild(allChip);
  for (const status of AUDIT_STATUSES) {
    const chip = el("button", { class: "chip", text: status });
    chip.dataset.status = status;
    chip.style.borderLeft = `2px solid var(--audit-status-${status}, var(--border))`;
    chip.addEventListener("click", () => {
      auditFilter.status = auditFilter.status === status ? null : status;
      syncAuditControls();
      const hash = buildAuditHash();
      window.location.hash = hash;
    });
    container.appendChild(chip);
  }
  syncAuditControls();
}

function wireAuditSearch() {
  const input = $("audit-search");
  if (!input) return;
  let debounce = null;
  input.addEventListener("input", (e) => {
    auditFilter.q = e.target.value.trim();
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => {
      const hash = buildAuditHash();
      if (window.location.hash !== hash) {
        window.location.hash = hash;
      } else {
        refreshDashboard();
      }
    }, 250);
  });
}

const AUDIT_COLUMNS = [
  { key: "time", label: "time" },
  { key: "role", label: "role" },
  { key: "tool", label: "tool" },
  { key: "command", label: "command" },
  { key: "target", label: "target" },
  { key: "status", label: "status" },
  { key: "exit", label: "exit", num: true },
  { key: "duration", label: "duration", num: true },
];

function renderAudit(events) {
  const body = $("audit-body");
  if (!body) return;
  $("audit-count").textContent = `${events.length}`;

  if (events.length === 0) {
    syncNodes(body, [el("div", { class: "empty-state" }, [
      el("div", { class: "icon", text: "✧" }),
      el("div", { class: "text", text: "No audit events match the current filter." }),
    ])]);
    return;
  }

  let table = body.querySelector("table.scoreboard-table");
  let tbody;
  if (!table) {
    table = el("table", { class: "scoreboard-table" });
    const thead = el("thead");
    const headRow = el("tr");
    for (const col of AUDIT_COLUMNS) {
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
  for (const ev of events) {
    const exit = ev.exit_code;
    const exitClass = exit != null && exit !== 0 ? "num exit-fail" : "num";
    const tool = ev.tool_name || "-";
    const target = ev.target_id || ev.target_type || "-";
    const cmd = ev.subcommand ? `${ev.command} ${ev.subcommand}` : ev.command;
    const tr = el("tr", { class: "audit-row", title: `event ${ev.id}` });
    tr.dataset.key = `audit-${ev.id}`;
    tr.dataset.hash = `${ev.id}-${ev.status}-${exit}`;
    tr.appendChild(el("td", { text: fmtTimestamp(ev.timestamp) }));
    tr.appendChild(el("td", { text: ev.role || "-" }));
    tr.appendChild(el("td", { text: tool }));
    tr.appendChild(el("td", { text: cmd }));
    tr.appendChild(el("td", { text: target, title: target }));
    const statusTd = el("td");
    statusTd.appendChild(el("span", { class: `audit-status ${ev.status}`, text: ev.status }));
    tr.appendChild(statusTd);
    tr.appendChild(el("td", { class: exitClass, text: exit == null ? "-" : String(exit) }));
    tr.appendChild(el("td", { class: "num", text: fmtDuration(ev.duration_ms) }));
    if (expandedAuditIds.has(ev.id)) tr.classList.add("expanded");
    tr.addEventListener("click", () => {
      if (expandedAuditIds.has(ev.id)) expandedAuditIds.delete(ev.id);
      else expandedAuditIds.add(ev.id);
      renderAudit(lastAudit);
    });
    frag.appendChild(tr);

    if (expandedAuditIds.has(ev.id)) {
      frag.appendChild(buildAuditDetailRow(ev));
    }
  }
  syncNodes(tbody, Array.from(frag.children));
}

function buildAuditDetailRow(ev) {
  const tr = el("tr", { class: "audit-detail-row" });
  tr.dataset.key = `audit-detail-${ev.id}`;
  tr.dataset.hash = JSON.stringify(ev);
  const td = el("td");
  td.colSpan = AUDIT_COLUMNS.length;
  td.addEventListener("click", (e) => e.stopPropagation());

  const meta = el("div", { class: "audit-detail-meta" });
  const addMeta = (label, value) => {
    if (value == null || value === "") return;
    meta.appendChild(el("span", {}, [
      el("span", { class: "label", text: `${label}:` }),
      el("span", { class: "value", text: String(value) }),
    ]));
  };
  const addMetaLink = (label, value, href) => {
    if (value == null || value === "") return;
    const link = el("a", { class: "value audit-detail-link", text: String(value) });
    link.href = href;
    meta.appendChild(el("span", {}, [
      el("span", { class: "label", text: `${label}:` }),
      link,
    ]));
  };
  addMeta("execution_id", ev.execution_id);
  addMeta("session_id", ev.session_id);
  addMeta("task_id", ev.task_id || "-");
  if (ev.job_run_id) {
    addMetaLink(
      "job_run_id",
      ev.job_run_id,
      `#runs/${encodeURIComponent(ev.job_run_id)}`,
    );
  } else {
    addMeta("job_run_id", "-");
  }
  addMeta("activity_id", ev.activity_id || "-");
  if (ev.step_index != null) {
    addMeta("step_index", ev.step_index);
  }
  addMeta("host", ev.host);
  addMeta("pid", ev.pid);
  addMeta("cwd", ev.working_directory);
  addMeta("timestamp", fmtAbsTime(ev.timestamp));
  td.appendChild(meta);

  if (ev.arguments_json) {
    const block = el("div", { class: "audit-detail-block" });
    block.appendChild(el("div", { class: "label", text: "arguments" }));
    let pretty = ev.arguments_json;
    try {
      pretty = JSON.stringify(JSON.parse(ev.arguments_json), null, 2);
    } catch (_) {
      /* leave raw */
    }
    block.appendChild(el("pre", { text: pretty }));
    td.appendChild(block);
  }
  if (ev.stderr_truncated) {
    const block = el("div", { class: "audit-detail-block" });
    block.appendChild(el("div", { class: "label", text: "stderr (truncated)" }));
    block.appendChild(el("pre", { text: ev.stderr_truncated }));
    td.appendChild(block);
  }
  if (ev.stdout_truncated) {
    const block = el("div", { class: "audit-detail-block" });
    block.appendChild(el("div", { class: "label", text: "stdout (truncated)" }));
    block.appendChild(el("pre", { text: ev.stdout_truncated }));
    td.appendChild(block);
  }
  if (ev.error_message) {
    const block = el("div", { class: "audit-detail-block" });
    block.appendChild(el("div", { class: "label", text: "error" }));
    block.appendChild(el("pre", { text: ev.error_message }));
    td.appendChild(block);
  }

  tr.appendChild(td);
  return tr;
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

buildChips();
wireSearch();
wireLearningSearch();
wireAdrSearch();
wireFrictionSearch();
buildAuditChips();
wireAuditSearch();
$("refresh-btn").addEventListener("click", refreshDashboard);
initTabs();

// -- Log Tail Panel --
let logStream = null;
let logBuffered = [];
let logFollowTail = true;
let logRows = []; // Keep track to enforce max 200 after 250 limit
let activeLogFilters = new Set(["all"]);
let logPanelResizeWired = false;

function fitLogPanelToViewport() {
  const panel = $("log-panel");
  if (!panel) return;
  const top = panel.getBoundingClientRect().top;
  const available = Math.floor(window.innerHeight - top - 24);
  if (available <= 0) return;
  panel.style.setProperty("--log-panel-height", `${available}px`);
}

function wireLogPanelResize() {
  if (logPanelResizeWired) return;
  logPanelResizeWired = true;
  const scheduleFit = () => requestAnimationFrame(fitLogPanelToViewport);
  window.addEventListener("resize", scheduleFit);
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", scheduleFit);
  }
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

function initLogTail() {
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

initLogTail();

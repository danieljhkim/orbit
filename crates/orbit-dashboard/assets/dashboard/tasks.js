// Orbit dashboard task-domain rendering and actions.
// Pure vanilla JS, split into ES modules with no build step.

import { el, priorityCell, statusPill, patchJson, syncNodes } from './common.js';

const $ = (id) => document.getElementById(id);

let lastCrewPayload = { default_crew: null, crews: [] };
let expandedTaskIds = new Set();
let taskActionNotice = null;
let crewUpdateErrors = new Map();
let pinnedExternalTask = null;

function taskList(context) {
  return context && typeof context.getTasks === "function" ? context.getTasks() : [];
}

function searchQueryValue(context) {
  return context && typeof context.getSearchQuery === "function"
    ? context.getSearchQuery()
    : "";
}

function activeStatusSet(context) {
  return context && typeof context.getActiveStatuses === "function"
    ? context.getActiveStatuses()
    : new Set();
}

function setSearchQuery(context, value) {
  if (context && typeof context.setSearchQuery === "function") context.setSearchQuery(value);
}

function setActiveStatuses(context, statuses) {
  if (context && typeof context.setActiveStatuses === "function") context.setActiveStatuses(statuses);
}

function statusOrder(context) {
  return context && Array.isArray(context.statusOrder) ? context.statusOrder : [];
}

function statusUpdateTargets(context) {
  return context && Array.isArray(context.statusUpdateTargets)
    ? context.statusUpdateTargets
    : [];
}

function fmtAbsTimeValue(context, value) {
  return context && typeof context.fmtAbsTime === "function"
    ? context.fmtAbsTime(value)
    : (value || "-");
}

function refreshTasks(context) {
  return context && typeof context.refreshDashboard === "function"
    ? context.refreshDashboard()
    : Promise.resolve();
}

export function normalizeCrewPayload(payload) {
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

export function cacheCrewPayload(payload) {
  lastCrewPayload = normalizeCrewPayload(payload);
  return lastCrewPayload;
}

export function hasCrewOptions() {
  return Array.isArray(lastCrewPayload.crews) && lastCrewPayload.crews.length > 0;
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

function applyUpdatedTask(updatedTask, context) {
  if (context && typeof context.replaceTask === "function") {
    context.replaceTask(updatedTask);
  }
}

function filterTasks(tasks, context) {
  const q = searchQueryValue(context);
  const activeStatuses = activeStatusSet(context);
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
  ["complexity", "complexity"],
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

export function copyTaskIdWithNotice(taskId, context) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(taskId).catch(() => {});
  }
  taskActionNotice = `${taskId} is not in the filtered task list; copied ID`;
  renderTasks(taskList(context), context);
}

function findTaskRow(taskId) {
  return Array.from(document.querySelectorAll("#tasks-body .row"))
    .find((row) => row.dataset.key === `task-${taskId}`) || null;
}

export function openVisibleTask(taskId, context) {
  const visible = filterTasks(taskList(context), context).some((task) => task.id === taskId);
  if (!visible) {
    copyTaskIdWithNotice(taskId, context);
    return;
  }
  expandedTaskIds.add(taskId);
  renderTasks(taskList(context), context);
  requestAnimationFrame(() => {
    const row = findTaskRow(taskId);
    if (!row) return;
    row.scrollIntoView({ behavior: "smooth", block: "center" });
    row.classList.add("data-changed");
    setTimeout(() => row.classList.remove("data-changed"), 1200);
  });
}

/* Global ID resolver support (ORB-00211): allow rendering detail for a task whose status
   is outside the active chip filter (done/rejected/archived) without adding it to the
   bulk-loaded list or mutating DASHBOARD_TASK_STATUSES. The pinned detail appears in a
   highlighted block at top of #tasks-body; auto-clears if user later enables matching chip. */
export function setPinnedExternalTask(task, context) {
  if (!task || !task.id) return;
  pinnedExternalTask = { task, id: task.id };
}

export function clearPinnedExternalTask(context) {
  pinnedExternalTask = null;
  if (context) renderTasks(taskList(context), context);
}

function refreshChips(context) {
  for (const chip of document.querySelectorAll("#task-filter .chip")) {
    const status = chip.dataset.status;
    const isAll = chip.dataset.role === "all";
    const activeStatuses = activeStatusSet(context);
    const allOn = activeStatuses.size === statusOrder(context).length;
    const on = isAll ? allOn : activeStatuses.has(status);
    chip.classList.toggle("active", on);
  }
}

export function buildChips(context) {
  const container = $("task-filter");
  container.innerHTML = "";
  const allChip = el("button", { class: "chip", text: "all" });
  allChip.dataset.role = "all";
  allChip.addEventListener("click", () => {
    setActiveStatuses(context, new Set(statusOrder(context)));
    refreshChips(context);
    renderTasks(taskList(context), context);
  });
  container.appendChild(allChip);
  for (const status of statusOrder(context)) {
    const chip = el("button", { class: "chip", text: status });
    chip.dataset.status = status;
    chip.style.borderLeft = `2px solid var(--status-${status}, var(--border))`;
    chip.addEventListener("click", () => {
      const activeStatuses = activeStatusSet(context);
      if (activeStatuses.has(status)) {
        activeStatuses.delete(status);
      } else {
        activeStatuses.add(status);
      }
      refreshChips(context);
      renderTasks(taskList(context), context);
    });
    container.appendChild(chip);
  }
  refreshChips(context);
}

export function wireSearch(context) {
  $("task-search").addEventListener("input", (e) => {
    setSearchQuery(context, e.target.value.trim().toLowerCase());
    renderTasks(taskList(context), context);
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

function buildRelations(relations, context) {
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
        openVisibleTask(target, context);
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

function buildReviewThreads(threads, context) {
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
        document.createTextNode(`[${fmtAbsTimeValue(context, msg.at)}] `),
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

function buildTaskDetail(task, context) {
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
    addField(leftCol, "review threads", buildReviewThreads(task.review_threads, context), true, true);
  }

  if (Array.isArray(task.artifacts) && task.artifacts.length > 0) {
    addField(leftCol, "artifacts", buildArtifacts(task), true, true);
  }

  if (Array.isArray(task.tags) && task.tags.length > 0) {
    rightCol.appendChild(buildTagRow(task.tags));
  }

  // Details before Assignment (per task: read-only meta first, mutating control second).
  // Assignment contains *only* the crew dropdown (complexityCell removed; it lives in
  // TASK_META_FIELDS inside Details).
  const meta = el("div", { class: "meta-list" });
  let metaCount = 0;
  for (const [key, label] of TASK_META_FIELDS) {
    const v = task[key];
    if (v == null || v === "") continue;
    const display = key.endsWith("_at") ? fmtAbsTimeValue(context, v) : String(v);
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

  const assignment = el("div", { class: "assignment" });
  assignment.appendChild(buildCrewUpdateControl(task, context));
  addField(rightCol, "assignment", assignment);

  if (Array.isArray(task.external_refs) && task.external_refs.length > 0) {
    addField(rightCol, "external refs", buildExternalRefs(task.external_refs));
  }

  if (Array.isArray(task.relations) && task.relations.length > 0) {
    const relations = buildRelations(task.relations, context);
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
        document.createTextNode(`[${fmtAbsTimeValue(context, h.at)}] `),
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
        document.createTextNode(`[${fmtAbsTimeValue(context, c.at)}] `),
        el("span", { class: "author", text: c.by || "?" }),
        document.createTextNode(`: ${c.message || ""}`),
      ]);
      wrap.appendChild(line);
    }
    addField(rightCol, "comments", wrap);
  }

  detail.appendChild(leftCol);
  detail.appendChild(rightCol);
  detail.appendChild(buildActionsRow(task, detail, context));

  return detail;
}

const APPROVE_STATUSES = new Set(["proposed", "review"]);
const REJECT_STATUSES = new Set(["proposed", "review", "backlog"]);

function buildActionsRow(task, detail, context) {
  const actions = el("div", { class: "actions" });
  if (APPROVE_STATUSES.has(task.status)) {
    const btn = el("button", { class: "action approve", text: "approve" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      runAction(task, "approve", detail, null, btn, context);
    });
    actions.appendChild(btn);
  }
  if (REJECT_STATUSES.has(task.status)) {
    const btn = el("button", { class: "action reject", text: "reject" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      showRejectForm(task, detail, actions, context);
    });
    actions.appendChild(btn);
  }
  if (task.status !== "archived") {
    const btn = el("button", { class: "action archive", text: "archive" });
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (window.confirm(`Archive task ${task.id}?`)) {
        runAction(task, "archive", detail, null, btn, context);
      }
    });
    actions.appendChild(btn);
  }
  const statusSelect = buildStatusUpdateControl(task, detail, context);
  if (statusSelect) actions.appendChild(statusSelect);
  return actions;
}

function buildStatusUpdateControl(task, detail, context) {
  const targets = statusUpdateTargets(context).filter((status) => status !== task.status);
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
      context,
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

function buildCrewUpdateControl(task, context) {
  const cell = el("span", { class: "crew-cell" });
  const select = el("select", {
    class: "task-crew-select mono",
    title: `Update crew for ${task.id}`,
  });
  const currentValue = explicitCrewValue(task);
  select.dataset.currentValue = currentValue;

  const defaultOption = el("option", {
    text: resolvedCrewName(task),
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
    updateTaskCrew(task, select, context);
  });

  cell.appendChild(select);
  const error = crewUpdateErrors.get(task.id);
  if (error) {
    cell.appendChild(el("span", { class: "crew-error", text: error }));
  }
  return cell;
}

async function updateTaskCrew(task, select, context) {
  const previousValue = select.dataset.currentValue || "";
  const nextValue = select.value || "";
  if (nextValue === previousValue) return;

  crewUpdateErrors.delete(task.id);
  select.disabled = true;
  try {
    const updatedTask = await patchJson(`/api/tasks/${encodeURIComponent(task.id)}`, {
      crew: nextValue || null,
    });
    applyUpdatedTask(updatedTask, context);
    crewUpdateErrors.delete(task.id);
    renderTasks(taskList(context), context);
  } catch (error) {
    select.value = previousValue;
    select.dataset.currentValue = previousValue;
    select.disabled = false;
    crewUpdateErrors.set(
      task.id,
      `crew update failed: ${error.message || String(error)}`,
    );
    renderTasks(taskList(context), context);
    console.error(error);
  }
}

function showRejectForm(task, detail, actions, context) {
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
    runAction(task, "reject", detail, { note }, submit, context);
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

async function runAction(task, kind, detail, body, btnNode, context, opts = {}) {
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
    await refreshTasks(context);
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

export function renderTasks(tasks, context) {
  const body = $("tasks-body");
  if (!body) return;

  // Auto-clear pinned external (global resolver) if its status+search now makes it
  // visible in the normal filtered list (user enabled the chip or cleared search).
  if (pinnedExternalTask && pinnedExternalTask.task) {
    const p = pinnedExternalTask.task;
    const q = (searchQueryValue(context) || "").toLowerCase();
    const act = activeStatusSet(context);
    const matchesQ = !q || (p.id && p.id.toLowerCase().includes(q)) || (p.title && p.title.toLowerCase().includes(q));
    if (act.has(p.status) && matchesQ) {
      pinnedExternalTask = null;
    }
  }

  const frag = document.createDocumentFragment();
  const notice = takeTaskActionNotice();

  // Render pinned external task detail (for statuses outside active filter) at top.
  if (pinnedExternalTask && pinnedExternalTask.task) {
    const ptask = pinnedExternalTask.task;
    const pRow = el("div", {
      class: "row pinned-external",
      title: `${ptask.title} (global resolver; status ${ptask.status})`
    }, [
      el("span", { class: "id mono", text: ptask.id }),
      el("span", { class: "title", text: ptask.title }),
      statusPill(ptask.status),
    ]);
    pRow.addEventListener("click", (e) => {
      e.stopPropagation();
      if (navigator.clipboard) navigator.clipboard.writeText(ptask.id).catch(() => {});
    });
    const pDetail = buildTaskDetail(ptask, context);
    // Dismiss button to close the global detail without affecting chips
    const dismiss = el("button", { class: "action", text: "Close" });
    dismiss.title = "Dismiss global task detail";
    dismiss.addEventListener("click", (ev) => {
      ev.stopPropagation();
      pinnedExternalTask = null;
      renderTasks(taskList(context), context);
    });
    let acts = pDetail.querySelector(".actions");
    if (!acts) {
      acts = el("div", { class: "actions" });
      pDetail.appendChild(acts);
    }
    acts.appendChild(dismiss);
    const pWrap = el("div", { class: "pinned-task-wrap" }, [pRow, pDetail]);
    pWrap.dataset.key = `pinned-${ptask.id}`;
    frag.appendChild(pWrap);
  }

  const filtered = filterTasks(tasks, context);
  $("tasks-count").textContent =
    filtered.length === tasks.length
      ? `${tasks.length}`
      : `${filtered.length}/${tasks.length}`;
  if (filtered.length === 0 && frag.children.length === 0) {
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

  // Column header strip (once, before first group-header). Uses .row.header so grid
  // (and all @media overrides) are identical to data rows; labels sit over ID/Title/Priority/Type.
  const colHeader = el("div", { class: "row header" }, [
    el("span", { class: "id", text: "ID" }),
    el("span", { class: "title", text: "Title" }),
    el("span", { class: "priority", text: "Priority" }),
    el("span", { class: "type", text: "Type" }),
  ]);
  colHeader.dataset.key = "task-col-header";
  frag.appendChild(colHeader);

  for (const t of filtered) {
    if (!groups.has(t.status)) groups.set(t.status, []);
    groups.get(t.status).push(t);
  }
  const order = statusOrder(context);
  const ordered = order.filter((s) => groups.has(s)).concat(
    [...groups.keys()].filter((s) => !order.includes(s)),
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
      row.dataset.hash = `${t.id}-${t.title}-${t.priority}-${t.type}-${t.complexity || ""}-${t.crew || ""}-${t.resolved_crew || ""}-${crewOptionsSignature()}-${crewUpdateErrors.get(t.id) || ""}-${expandedTaskIds.has(t.id)}`;
      row.addEventListener("click", () => {
        const toggle = () => {
          if (expandedTaskIds.has(t.id)) expandedTaskIds.delete(t.id);
          else expandedTaskIds.add(t.id);
          renderTasks(taskList(context), context);
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
        const detail = buildTaskDetail(t, context);
        detail.dataset.key = `detail-${t.id}`;
        // Diff by full task object stringified
        detail.dataset.hash = JSON.stringify(t);
        frag.appendChild(detail);
      }
    }
  }
  syncNodes(body, Array.from(frag.children));
}


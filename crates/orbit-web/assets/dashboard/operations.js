// Routine-definition, host sweep-clock, and auto-task operations [ORB-10875, ORB-10876].

import { el, fetchJson, getWorkspace, postJson } from './common.js';

const $ = (id) => document.getElementById(id);
const pendingOperations = new Set();
const UNCONDITIONAL_MINT_WARNING = "Manual mint ignores this definition's schedule, enabled flag, and scheduler dedupe policy.";
let lastOperations = null;
let lastAutoTasks = null;
let context = null;

export function initOperations(nextContext) {
  context = nextContext;
}

function selectedWorkspace() {
  const selected = getWorkspace();
  if (!selected) return null;
  return context.getWorkspaces().find((workspace) => workspace.id === selected) || null;
}

function selectedWorkspaceName() {
  return selectedWorkspace()?.name || null;
}

function workspaceReadOnlyReason() {
  const selected = getWorkspace();
  if (!selected) return "All-workspace mode is read-only. Select one workspace; auto-task definitions are workspace-scoped.";
  const workspace = selectedWorkspace();
  if (!workspace) return `Workspace ${selected} is not a concrete active selection.`;
  if (workspace.status && workspace.status !== "active") {
    return `Workspace ${workspace.name} is inactive; select an active workspace.`;
  }
  return "";
}

function feedback(id, kind, message) {
  const node = $(id);
  if (!node) return;
  node.className = `operation-feedback ${kind || ""}`;
  node.textContent = message || "";
}

function time(value) {
  if (!value) return "Not observed";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? String(value) : context.formatAbsoluteTime(value);
}

function field(label, value) {
  return el("div", { class: "operation-field" }, [
    el("span", { class: "operation-field-label", text: label }),
    el("span", { class: "operation-field-value", text: value == null || value === "" ? "—" : String(value) }),
  ]);
}

function controlReason(payload, routine) {
  if (!getWorkspace()) return "Select one workspace before changing routine or clock state.";
  if (!payload.controls_authorized) return "Controls require an authorized operator session.";
  if (routine && !routine.pinned_to_host) return `Select pinned host ${payload.host_id} before changing this routine.`;
  return "";
}

function routineButton(payload, routine) {
  const nextEnabled = !routine.enabled;
  const key = `routine:${routine.name}`;
  const reason = controlReason(payload, routine);
  const button = el("button", {
    class: `operation-button ${nextEnabled ? "enable" : "disable"}`,
    text: pendingOperations.has(key) ? "Pending…" : nextEnabled ? "Enable" : "Disable",
    title: reason || `${nextEnabled ? "Enable" : "Disable"} ${routine.target}`,
  });
  button.type = "button";
  button.disabled = Boolean(reason) || pendingOperations.has(key);
  button.addEventListener("click", async () => {
    if (pendingOperations.has(key)) return;
    pendingOperations.add(key);
    feedback("routine-operation-feedback", "pending", `${nextEnabled ? "Enabling" : "Disabling"} ${routine.name} → ${routine.target}…`);
    renderOperations(payload);
    try {
      const result = await postJson("/api/routines/toggle", {
        name: routine.name,
        source: routine.source,
        target: routine.target,
        host_id: payload.host_id,
        expected_enabled: routine.enabled,
        enabled: nextEnabled,
      });
      feedback("routine-operation-feedback", "success", `${result.message}: ${routine.name} → ${routine.target}.`);
      await fetchAndRenderOperations();
    } catch (error) {
      feedback("routine-operation-feedback", "error", `Routine change failed: ${error.message}`);
    } finally {
      pendingOperations.delete(key);
      if (lastOperations) renderOperations(lastOperations);
    }
  });
  return button;
}

function renderOperations(payload) {
  lastOperations = payload;
  const workspace = selectedWorkspaceName();
  const routines = workspace
    ? (payload.routines || []).filter((routine) => routine.source === workspace)
    : [];
  const body = $("routines-body");
  body.textContent = "";
  if (!workspace) {
    body.appendChild(el("div", { class: "operations-readonly-note", text: `All-workspace mode is read-only. Select one workspace; this host is already resolved as ${payload.host_id}.` }));
  }
  if (routines.length === 0) {
    body.appendChild(el("div", { class: "empty-state", text: workspace ? "No routines are defined by this workspace." : "Select a workspace to list its routines." }));
  }
  for (const routine of routines) {
    const fire = routine.last_fire;
    const state = routine.enabled ? (routine.effective ? "enabled" : "blocked") : "disabled";
    const card = el("article", { class: "operation-card routine-card" });
    card.append(
      el("div", { class: "operation-card-title" }, [
        el("div", {}, [el("strong", { text: routine.name }), el("span", { class: `operation-state ${state}`, text: state })]),
        routineButton(payload, routine),
      ]),
      el("div", { class: "operation-target mono", text: routine.target }),
      el("div", { class: "operation-grid" }, [
        field("Source workspace", routine.source),
        field("Schedule", routine.cron),
        field("Host pin", (routine.hosts || []).join(", ") || "Local host"),
        field("Last evaluation", time(routine.last_evaluated_slot || routine.first_observed_at)),
        field("Next evaluation", time(routine.next_due)),
        field("Last fire", fire ? time(fire.finished_at || fire.started_at) : "Never"),
        field("Linked run / outcome", fire ? `${fire.run_id || "No run"} · ${fire.state}` : "No fire recorded"),
      ]),
    );
    if (routine.description) card.appendChild(el("p", { class: "operation-description", text: routine.description }));
    const reason = controlReason(payload, routine);
    if (reason) card.appendChild(el("p", { class: "operation-control-note", text: reason }));
    body.appendChild(card);
  }
  $("routines-count").textContent = workspace ? `${routines.length} · ${workspace}` : "read-only";
  renderClock(payload);
}

function clockButton(payload, action, label) {
  const key = "clock:service";
  const reason = controlReason(payload);
  const button = el("button", { class: "operation-button", text: pendingOperations.has(key) ? "Pending…" : label, title: reason });
  button.type = "button";
  button.disabled = Boolean(reason) || pendingOperations.has(key);
  button.addEventListener("click", async () => {
    if (pendingOperations.has(key)) return;
    const verb = action === "enable" ? "Start" : "Stop";
    if (!window.confirm(`${verb} the ${payload.clock.provider} sweep clock on ${payload.host_id}? This does not change any routine definition.`)) return;
    pendingOperations.add(key);
    feedback("clock-operation-feedback", "pending", `${action === "enable" ? "Starting" : "Stopping"} the host sweep clock…`);
    renderClock(payload);
    try {
      const result = await postJson("/api/routines/clock", {
        action,
        host_id: payload.host_id,
        expected_enabled: payload.clock.enabled,
        expected_cadence_seconds: payload.clock.configured_cadence_seconds,
      });
      feedback("clock-operation-feedback", "success", `${result.message} on ${payload.host_id}.`);
      await fetchAndRenderOperations();
    } catch (error) {
      feedback("clock-operation-feedback", "error", `Clock service change failed: ${error.message}`);
    } finally {
      pendingOperations.delete(key);
      if (lastOperations) renderClock(lastOperations);
    }
  });
  return button;
}

function renderClock(payload) {
  const clock = payload.clock;
  const body = $("clock-body");
  body.textContent = "";
  const reason = controlReason(payload);
  if (reason) body.appendChild(el("div", { class: "operations-readonly-note", text: reason }));
  body.append(
    el("div", { class: "operation-clock-summary" }, [
      el("span", { class: `operation-state ${clock.health}`, text: clock.health }),
      el("span", { class: "mono", text: clock.provider }),
      el("span", { text: clock.enabled ? "service enabled" : "service paused" }),
    ]),
    el("div", { class: "operation-grid" }, [
      field("Configured cadence", `${clock.configured_cadence_seconds}s`),
      field("Effective cadence", clock.effective_cadence_seconds ? `${clock.effective_cadence_seconds}s` : "Inactive"),
      field("Loaded", clock.loaded ? "Yes" : "No"),
      field("Running / waiting", clock.running == null ? "Provider does not expose" : clock.running ? "Yes" : "No"),
      field("Last tick", clock.last_tick_at || "Provider does not expose"),
      field("Next expected tick", clock.next_tick_at || (clock.schedulable ? "Armed; exact time unavailable" : "Not scheduled")),
    ]),
  );
  if (clock.health_issue) body.appendChild(el("p", { class: "operation-control-note error", text: clock.health_issue }));
  const actions = el("div", { class: "operation-clock-actions" });
  actions.appendChild(clockButton(payload, clock.enabled ? "disable" : "enable", clock.enabled ? "Pause clock" : "Enable clock"));
  const cadence = el("select", { class: "operation-cadence", title: "Clock cadence" });
  for (const seconds of [60, 300, 900, 1800, 3600]) {
    const option = el("option", { text: seconds === 60 ? "Every minute" : `Every ${seconds / 60} minutes` });
    option.value = String(seconds);
    option.selected = seconds === clock.configured_cadence_seconds;
    cadence.appendChild(option);
  }
  cadence.disabled = Boolean(reason) || pendingOperations.has("clock:cadence");
  const apply = el("button", { class: "operation-button secondary", text: pendingOperations.has("clock:cadence") ? "Pending…" : "Apply cadence", title: "Reload cadence without changing whether the clock is enabled" });
  apply.type = "button";
  apply.disabled = Boolean(reason) || pendingOperations.has("clock:cadence") || Number(cadence.value) === clock.configured_cadence_seconds;
  cadence.addEventListener("change", () => { apply.disabled = Boolean(reason) || Number(cadence.value) === clock.configured_cadence_seconds; });
  apply.addEventListener("click", async () => {
    const key = "clock:cadence";
    if (pendingOperations.has(key)) return;
    pendingOperations.add(key);
    feedback("clock-operation-feedback", "pending", `Changing cadence to ${cadence.value}s; clock service state remains separate…`);
    renderClock(payload);
    try {
      const result = await postJson("/api/routines/clock", {
        action: "set_cadence",
        host_id: payload.host_id,
        expected_enabled: clock.enabled,
        expected_cadence_seconds: clock.configured_cadence_seconds,
        cadence_seconds: Number(cadence.value),
      });
      feedback("clock-operation-feedback", "success", `${result.message}; service is ${result.clock.enabled ? "enabled" : "paused"}.`);
      await fetchAndRenderOperations();
    } catch (error) {
      feedback("clock-operation-feedback", "error", `Cadence change failed: ${error.message}`);
    } finally {
      pendingOperations.delete(key);
      if (lastOperations) renderClock(lastOperations);
    }
  });
  actions.append(cadence, apply);
  body.appendChild(actions);
  $("clock-host").textContent = payload.host_id || "unknown host";
}

function autoTaskControlReason(payload) {
  const workspaceReason = workspaceReadOnlyReason();
  if (workspaceReason) return workspaceReason;
  if (payload && payload.read_only_reason) return payload.read_only_reason;
  if (payload && payload.controls_authorized === false) return "Controls require an authorized operator session.";
  return "";
}

function lastEvaluationText(definition) {
  const evaluation = definition.last_evaluation;
  if (!evaluation) return "Never evaluated";
  if (evaluation.kind === "fired") {
    const task = evaluation.last_task_id ? ` · ${evaluation.last_task_id}` : "";
    return `${time(evaluation.last_fired_at || evaluation.last_slot)}${task}`;
  }
  return `Baselined ${time(evaluation.baseline_at)}; no fire yet`;
}

function autoTaskToggleButton(payload, definition) {
  const nextEnabled = !definition.enabled;
  const key = `auto-task-toggle:${definition.name}`;
  const reason = autoTaskControlReason(payload);
  const verb = nextEnabled ? "Enable" : "Disable";
  const button = el("button", {
    class: `operation-button ${nextEnabled ? "enable" : "disable"}`,
    text: pendingOperations.has(key) ? "Pending…" : verb,
    title: reason || `${verb} ${definition.name}`,
  });
  button.type = "button";
  button.disabled = Boolean(reason) || pendingOperations.has(key);
  button.addEventListener("click", async () => {
    if (pendingOperations.has(key)) return;
    const workspace = selectedWorkspace();
    const summary = definition.template_summary || definition.template?.title || definition.name;
    if (!window.confirm(`${verb} auto-task "${definition.name}" in workspace "${workspace?.name || workspace?.id}"?\n\nTarget: ${summary}\nThis writes the definition's enabled field.`)) return;
    pendingOperations.add(key);
    feedback("auto-task-operation-feedback", "pending", `${verb === "Enable" ? "Enabling" : "Disabling"} ${definition.name} (${summary})…`);
    renderAutoTasks(payload);
    try {
      const result = await postJson("/api/auto-tasks/toggle", {
        name: definition.name,
        expected_enabled: definition.enabled,
        enabled: nextEnabled,
      });
      feedback("auto-task-operation-feedback", "success", `${result.message}: ${definition.name}.`);
      await fetchAndRenderAutoTasks();
    } catch (error) {
      feedback("auto-task-operation-feedback", "error", `Auto-task change failed: ${error.message}`);
    } finally {
      pendingOperations.delete(key);
      if (lastAutoTasks) renderAutoTasks(lastAutoTasks);
    }
  });
  return button;
}

function autoTaskMintButton(payload, definition) {
  const key = `auto-task-mint:${definition.name}`;
  const reason = autoTaskControlReason(payload);
  const button = el("button", {
    class: "operation-button secondary",
    text: pendingOperations.has(key) ? "Pending…" : "Mint now",
    title: reason || "Mint one task now, ignoring schedule, enabled, and dedupe",
  });
  button.type = "button";
  button.disabled = Boolean(reason) || pendingOperations.has(key);
  button.addEventListener("click", async () => {
    if (pendingOperations.has(key)) return;
    const workspace = selectedWorkspace();
    const template = definition.template || {};
    const duplicateLine = definition.may_create_open_duplicate
      ? "An open instance already exists; this will create another open task."
      : "No open instance is currently tagged for this definition.";
    const confirmText = [
      `Mint auto-task "${definition.name}" now in workspace "${workspace?.name || workspace?.id}"?`,
      "",
      `Resulting task: ${definition.template_summary || template.title || definition.name}`,
      `Crew: ${template.crew || "none"} · Status: ${template.status || "backlog"} · Priority: ${template.priority || "medium"}`,
      "",
      `WARNING: ${payload.unconditional_mint_warning || UNCONDITIONAL_MINT_WARNING}`,
      duplicateLine,
    ].join("\n");
    if (!window.confirm(confirmText)) return;
    pendingOperations.add(key);
    feedback("auto-task-operation-feedback", "pending", `Minting ${definition.name} now…`);
    renderAutoTasks(payload);
    try {
      const result = await postJson("/api/auto-tasks/mint", {
        name: definition.name,
        acknowledge_unconditional: true,
      });
      feedback("auto-task-operation-feedback", "success", `${result.message}`);
      await fetchAndRenderAutoTasks();
    } catch (error) {
      feedback("auto-task-operation-feedback", "error", `Manual mint failed: ${error.message}`);
    } finally {
      pendingOperations.delete(key);
      if (lastAutoTasks) renderAutoTasks(lastAutoTasks);
    }
  });
  return button;
}

function renderAutoTasks(payload) {
  lastAutoTasks = payload;
  const body = $("auto-tasks-body");
  if (!body) return;
  body.textContent = "";
  const workspaceReason = workspaceReadOnlyReason();
  const reason = workspaceReason || payload.read_only_reason || "";
  const workspace = selectedWorkspace();
  const definitions = workspace && !workspaceReason ? (payload.definitions || []) : [];
  if (reason) {
    body.appendChild(el("div", { class: "operations-readonly-note", text: reason }));
  }
  if (definitions.length === 0) {
    body.appendChild(el("div", {
      class: "empty-state",
      text: workspace && !workspaceReason ? "No auto-task definitions are defined by this workspace." : "Select a workspace to list its auto-task definitions.",
    }));
  }
  for (const definition of definitions) {
    const state = definition.enabled ? "enabled" : "disabled";
    const actions = el("div", { class: "operation-card-actions" }, [
      autoTaskToggleButton(payload, definition),
      autoTaskMintButton(payload, definition),
    ]);
    const card = el("article", { class: "operation-card auto-task-card" });
    card.append(
      el("div", { class: "operation-card-title" }, [
        el("div", {}, [
          el("strong", { text: definition.name }),
          el("span", { class: `operation-state ${state}`, text: state }),
        ]),
        actions,
      ]),
      el("div", { class: "operation-target mono", text: definition.template_summary || definition.template?.title || "" }),
      el("div", { class: "operation-grid" }, [
        field("Schedule", definition.schedule_summary || "—"),
        field("Dedupe", definition.dedupe === "always" ? "always fire" : "skip if open"),
        field("Last evaluation / mint", lastEvaluationText(definition)),
        field("Last minted task", definition.last_minted_task_id
          ? `${definition.last_minted_task_id}${definition.last_minted_task_status ? ` · ${definition.last_minted_task_status}` : ""}`
          : "None"),
        field("Next evaluation", time(definition.next_evaluation)),
        field("Open duplicate", definition.open_duplicate ? "Yes — mint will create another" : "No"),
      ]),
    );
    if (definition.description) {
      card.appendChild(el("p", { class: "operation-description", text: definition.description }));
    }
    card.appendChild(el("p", {
      class: "operation-control-note operation-mint-warning",
      text: payload.unconditional_mint_warning || UNCONDITIONAL_MINT_WARNING,
    }));
    const controlReason = autoTaskControlReason(payload);
    if (controlReason) card.appendChild(el("p", { class: "operation-control-note", text: controlReason }));
    body.appendChild(card);
  }
  const count = $("auto-tasks-count");
  if (count) count.textContent = workspace && !workspaceReason ? `${definitions.length} · ${workspace.name}` : "read-only";
}

function fetchAndRenderAutoTasks() {
  return fetchJson("/api/auto-tasks").then(renderAutoTasks);
}

export function fetchAndRenderOperations() {
  return Promise.all([
    fetchJson("/api/routines").then(renderOperations),
    fetchAndRenderAutoTasks().catch((error) => {
      feedback("auto-task-operation-feedback", "error", `Failed to load auto-tasks: ${error.message}`);
    }),
  ]);
}

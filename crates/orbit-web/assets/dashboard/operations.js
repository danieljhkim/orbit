// Routine-definition and host sweep-clock operations [ORB-10875].

import { el, fetchJson, getWorkspace, postJson } from './common.js';

const $ = (id) => document.getElementById(id);
const pendingOperations = new Set();
let lastOperations = null;
let context = null;

export function initOperations(nextContext) {
  context = nextContext;
}

function selectedWorkspaceName() {
  const selected = getWorkspace();
  return context.getWorkspaces().find((workspace) => workspace.id === selected)?.name || null;
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

export function fetchAndRenderOperations() {
  return fetchJson("/api/routines").then(renderOperations);
}

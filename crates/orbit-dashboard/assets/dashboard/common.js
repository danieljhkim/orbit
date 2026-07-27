const params = new URLSearchParams(window.location.search);

// ORB-00030: the dashboard can serve multiple workspaces. `currentWorkspace`
// (a workspace id, or null for the aggregate "all" view) is transparently
// appended as `?workspace=<id>` to every API request below, so individual view
// modules stay workspace-agnostic. Initialized from the URL for shareable links.
let currentWorkspace = params.get("workspace") || null;

export function getWorkspace() {
  return currentWorkspace;
}

export function setWorkspace(id) {
  currentWorkspace = id || null;
}

// ORB-00030/00039/00040: the dashboard can serve multiple workspaces. "Multi-
// workspace mode" is on when more than one workspace is servable; the aggregate
// ("All workspaces") view is that mode with no concrete workspace selected. In
// that view the per-workspace endpoints have no workspace to scope to and the
// backend `Ws` extractor 400s, so every view module (app.js, audit.js,
// scoreboard.js) guards its per-workspace fetches on isAggregateView() and
// renders a placeholder instead. This lives in the shared leaf module so all
// three modules query the same live predicate without a circular import.
let multiWorkspace = false;

export function setMultiWorkspace(value) {
  multiWorkspace = !!value;
}

export function isAggregateView() {
  return multiWorkspace && !currentWorkspace;
}

// Inline text shown in place of a per-workspace panel's body while the aggregate
// view is active, instead of erroring or holding stale content.
export const AGGREGATE_PANEL_PLACEHOLDER = "Select a workspace to view this panel";

export function renderPanelPlaceholder(bodyId) {
  const body = document.getElementById(bodyId);
  if (!body) return;
  const note = el("div", { class: "panel-placeholder", text: AGGREGATE_PANEL_PLACEHOLDER });
  note.dataset.key = "aggregate-placeholder";
  note.dataset.hash = "aggregate-placeholder";
  syncNodes(body, [note]);
}

// Append the selected workspace to an API path, unless one is already present
// (aggregate endpoints like /api/tasks/all are called with no workspace set).
export function withWorkspace(path) {
  if (!currentWorkspace || /[?&]workspace=/.test(path)) return path;
  const sep = path.includes("?") ? "&" : "?";
  return `${path}${sep}workspace=${encodeURIComponent(currentWorkspace)}`;
}

export function positiveIntParam(name, fallback) {
  const parsed = parseInt(params.get(name) || String(fallback), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function el(tag, opts = {}, children = []) {
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

export function statusPill(status) {
  const color = `var(--status-${status}, var(--fg))`;
  const pill = el("span", { class: "pill mono", text: status });
  pill.style.color = color;
  pill.style.borderLeft = `2px solid ${color}`;
  return pill;
}

export function priorityCell(p) {
  const node = el("span", { class: "priority mono", text: p });
  node.style.color = `var(--priority-${p}, var(--fg-dim))`;
  return node;
}

export function stateCell(state) {
  const node = el("span", { class: "mono", text: state });
  node.style.color = `var(--state-${state}, var(--fg-dim))`;
  return node;
}

export function fetchJson(path) {
  return fetch(withWorkspace(path), { headers: { accept: "application/json" } })
    .then(res => {
      if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
      return res.json();
    });
}

// ORB-10400: /api/tasks answers a paginated envelope
// `{ items, total, limit, truncated }` so a client can tell an empty result from
// a truncated window, while the /api/tasks/all aggregate still answers a bare
// array. Accept either shape rather than teaching each call site the difference.
export function listItems(payload) {
  if (Array.isArray(payload)) return payload;
  if (payload && Array.isArray(payload.items)) return payload.items;
  return [];
}

export function requestJson(path, method, body) {
  const headers = { accept: "application/json" };
  const opts = {
    method,
    headers,
  };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  return fetch(withWorkspace(path), opts).then(async (res) => {
    const text = await res.text();
    const body = text ? JSON.parse(text) : {};
    if (!res.ok) {
      throw new Error(body.error || `${path}: HTTP ${res.status}`);
    }
    return body;
  });
}

export function postJson(path, body) {
  return requestJson(path, "POST", body);
}

export function patchJson(path, body) {
  return requestJson(path, "PATCH", body);
}

export function syncNodes(container, newNodesArr) {
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

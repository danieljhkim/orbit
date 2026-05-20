const params = new URLSearchParams(window.location.search);

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

export function complexityCell(c) {
  const text = c ? String(c) : "—";
  const node = el("span", { class: "complexity mono", text });
  if (c) {
    node.style.color = `var(--complexity-${c}, var(--fg-dim))`;
  } else {
    node.style.color = "var(--fg-dim)";
    node.style.opacity = "0.55";
  }
  return node;
}

export function stateCell(state) {
  const node = el("span", { class: "mono", text: state });
  node.style.color = `var(--state-${state}, var(--fg-dim))`;
  return node;
}

export function fetchJson(path) {
  return fetch(path, { headers: { accept: "application/json" } })
    .then(res => {
      if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
      return res.json();
    });
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
  return fetch(path, opts).then(async (res) => {
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

// Cross-task review-thread panel for the dashboard.
//
// Lists threads from /api/review-threads with filters (status, author kind),
// supports reply / resolve / re-open in place, and surfaces an unread badge
// for agent-authored threads that appeared since the last visit to this tab.

import { el, fetchJson, postJson, syncNodes } from './common.js';

const $ = (id) => document.getElementById(id);

const SEEN_KEY = 'orbit-dashboard.review-threads.seen-agent-thread-ids';

let lastPayload = { items: [], stats: {} };
let statusFilter = 'open';
let authorKindFilter = 'both';
let activeThreadId = null;
let lastSeenAgentThreads = loadSeenAgentThreads();
let pendingErrors = new Map();

function loadSeenAgentThreads() {
  try {
    const raw = window.localStorage.getItem(SEEN_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return new Set(Array.isArray(parsed) ? parsed : []);
  } catch (_) {
    return new Set();
  }
}

function persistSeenAgentThreads() {
  try {
    window.localStorage.setItem(
      SEEN_KEY,
      JSON.stringify(Array.from(lastSeenAgentThreads)),
    );
  } catch (_) {
    // best-effort; localStorage may be unavailable
  }
}

function agentAuthoredThreadIds(items) {
  const ids = [];
  for (const item of items) {
    if (!item || !item.thread_id) continue;
    const lastKind = String(item.last_author_kind || '').toLowerCase();
    if (lastKind === 'agent') ids.push(item.thread_id);
  }
  return ids;
}

function computeUnreadCount(items) {
  let unread = 0;
  for (const id of agentAuthoredThreadIds(items)) {
    if (!lastSeenAgentThreads.has(id)) unread += 1;
  }
  return unread;
}

export function markCurrentAgentThreadsSeen() {
  let changed = false;
  for (const id of agentAuthoredThreadIds(lastPayload.items)) {
    if (!lastSeenAgentThreads.has(id)) {
      lastSeenAgentThreads.add(id);
      changed = true;
    }
  }
  if (changed) {
    persistSeenAgentThreads();
    refreshBadge();
    render();
  }
}

function refreshBadge() {
  const unreadBadge = $('threads-unread-badge');
  if (!unreadBadge) return;
  const unread = computeUnreadCount(lastPayload.items);
  if (unread > 0) {
    unreadBadge.style.display = '';
    unreadBadge.textContent = String(unread);
  } else {
    unreadBadge.style.display = 'none';
  }
}

function authorLabel(kind, family) {
  if (kind === 'agent') {
    return family ? `agent · ${family}` : 'agent';
  }
  return 'human';
}

function authorAvatar(kind, family) {
  if (kind !== 'agent') return 'HU';
  const label = String(family || 'agent').slice(0, 2).toUpperCase();
  return label || 'AG';
}

function fmtAbs(iso) {
  if (!iso) return '-';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fmtRelative(iso) {
  if (!iso) return '-';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const diff = Math.max(0, (Date.now() - d.getTime()) / 1000);
  if (diff < 60) return `${Math.floor(diff)}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function buildFilterControls() {
  const wrap = $('threads-filters');
  if (!wrap) return;
  wrap.innerHTML = '';

  const statusGroup = el('div', { class: 'thread-filter-group' });
  statusGroup.appendChild(el('span', { class: 'thread-filter-label', text: 'status' }));
  for (const value of ['open', 'resolved', 'all']) {
    const chip = el('button', { class: 'chip', text: value });
    chip.type = 'button';
    if (value === statusFilter) chip.classList.add('active');
    chip.addEventListener('click', () => {
      statusFilter = value;
      fetchAndRender().catch((err) => console.error(err));
    });
    statusGroup.appendChild(chip);
  }
  wrap.appendChild(statusGroup);

  const authorGroup = el('div', { class: 'thread-filter-group' });
  authorGroup.appendChild(el('span', { class: 'thread-filter-label', text: 'author' }));
  for (const [label, value] of [['both', 'both'], ['human', 'human'], ['agent', 'agent']]) {
    const chip = el('button', { class: 'chip', text: label });
    chip.type = 'button';
    if (value === authorKindFilter) chip.classList.add('active');
    chip.addEventListener('click', () => {
      authorKindFilter = value;
      fetchAndRender().catch((err) => console.error(err));
    });
    authorGroup.appendChild(chip);
  }
  wrap.appendChild(authorGroup);
}

function renderThreadStats(items, stats = {}) {
  const host = $('threads-stats');
  if (!host) return;
  const open = Number.isFinite(Number(stats.open)) ? Number(stats.open) : 0;
  const resolved = Number.isFinite(Number(stats.resolved)) ? Number(stats.resolved) : 0;
  const total = Number.isFinite(Number(stats.total)) ? Number(stats.total) : items.length;
  const latest = items
    .slice()
    .sort((a, b) => String(b.last_activity_at || '').localeCompare(String(a.last_activity_at || '')))[0];
  const unread = computeUnreadCount(items);
  const nodes = [
    statTile('open', String(open), `of ${total}`),
    statTile('resolved', String(resolved), `of ${total}`),
    statTile('unread', String(unread), 'agent replies'),
    statTile('last reply', latest ? fmtRelative(latest.last_activity_at) : '-', latest?.task_id || '-'),
  ];
  syncNodes(host, nodes);
}

function statTile(label, value, meta) {
  const node = el('div', { class: 'thread-stat' }, [
    el('span', { class: 'label', text: label }),
    el('div', { class: 'row' }, [
      el('span', { class: `value ${value === '0' ? 'dim' : ''}`, text: value }),
      el('span', { class: 'meta', text: meta }),
    ]),
  ]);
  node.dataset.key = `thread-stat-${label}`;
  node.dataset.hash = `${label}-${value}-${meta}`;
  return node;
}

function taskStatusBadge(status) {
  return el('span', { class: `thread-task-status ${status || 'unknown'}`, text: status || 'unknown' });
}

function threadStatusBadge(status) {
  return el('span', { class: `thread-status-pill ${status || 'open'}`, text: status || 'open' });
}

function activeItem(items) {
  if (items.length === 0) {
    activeThreadId = null;
    return null;
  }
  const current = items.find((item) => item.thread_id === activeThreadId);
  if (current) return current;
  activeThreadId = items[0].thread_id;
  return items[0];
}

function buildThreadList(items) {
  const list = el('div', { class: 'thread-list' });
  list.dataset.key = 'thread-list';
  list.dataset.hash = `${items.map((item) => `${item.thread_id}:${item.status}:${item.message_count}:${item.last_activity_at}:${item.thread_id === activeThreadId}`).join('|')}`;

  for (const item of items) {
    const unread = String(item.last_author_kind || '').toLowerCase() === 'agent'
      && !lastSeenAgentThreads.has(item.thread_id);
    const row = el('button', {
      class: `thread-row${item.thread_id === activeThreadId ? ' active' : ''}${unread ? ' unread' : ''}`,
      title: item.task_title || item.task_id,
    }, [
      el('div', { class: 'top' }, [
        el('span', { class: 'id', text: item.task_id || '-' }),
        taskStatusBadge(item.task_status || 'backlog'),
        el('span', { class: 'spacer' }),
        el('span', { class: 'when', text: fmtRelative(item.last_activity_at) }),
      ]),
      el('div', { class: 'title', text: item.task_title || item.task_id || item.thread_id }),
      el('div', { class: 'preview' }, [
        el('span', { class: 'by', text: `${authorLabel(item.last_author_kind, item.last_author_family)}:` }),
        document.createTextNode(item.body_preview || ''),
      ]),
      el('div', { class: 'meta' }, [
        el('span', { class: `author-kind ${item.last_author_family || 'human'}`, text: authorLabel(item.last_author_kind, item.last_author_family) }),
        el('span', { class: 'dot', text: '·' }),
        el('span', { text: `${item.message_count || 0} msgs` }),
        el('span', { class: 'dot', text: '·' }),
        threadStatusBadge(item.status || 'open'),
      ]),
    ]);
    row.type = 'button';
    row.dataset.key = `thread-row-${item.thread_id}`;
    row.dataset.hash = `${item.thread_id}-${item.status}-${item.message_count}-${item.last_activity_at}-${item.thread_id === activeThreadId}-${unread}`;
    row.addEventListener('click', () => {
      activeThreadId = item.thread_id;
      if (unread) {
        lastSeenAgentThreads.add(item.thread_id);
        persistSeenAgentThreads();
        refreshBadge();
      }
      render();
    });
    list.appendChild(row);
  }
  return list;
}

function buildMessageList(messages) {
  const list = el('div', { class: 'thread-messages' });
  for (const message of messages || []) {
    const kind = message.author_kind || 'human';
    const family = message.agent_family;
    const msg = el('div', { class: `thread-message ${kind}` }, [
      el('div', { class: 'avatar', text: authorAvatar(kind, family) }),
      el('div', { class: 'bubble' }, [
        el('div', { class: 'head' }, [
          el('span', { class: 'author', text: authorLabel(kind, family) }),
          el('span', { class: 'when', text: `${fmtRelative(message.at)} · ${fmtAbs(message.at)}` }),
        ]),
        el('div', { class: 'body', text: message.body || '' }),
      ]),
    ]);
    list.appendChild(msg);
  }
  return list;
}

function buildReplyForm(item) {
  const form = el('div', { class: 'thread-reply' });
  const textarea = el('textarea');
  textarea.placeholder = 'Reply to this thread';
  textarea.rows = 3;
  const submit = el('button', { class: 'btn send', text: 'send reply' });
  submit.type = 'button';
  const send = async () => {
    const body = (textarea.value || '').trim();
    if (!body) {
      textarea.focus();
      return;
    }
    submit.disabled = true;
    textarea.disabled = true;
    try {
      await postJson(
        `/api/tasks/${encodeURIComponent(item.task_id)}/review-threads/${encodeURIComponent(item.thread_id)}/reply`,
        { body },
      );
      pendingErrors.delete(item.thread_id);
      activeThreadId = item.thread_id;
      await fetchAndRender();
    } catch (err) {
      pendingErrors.set(item.thread_id, err.message || String(err));
      submit.disabled = false;
      textarea.disabled = false;
      render();
    }
  };
  submit.addEventListener('click', (e) => {
    e.stopPropagation();
    send();
  });
  textarea.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      send();
    }
  });
  form.appendChild(textarea);
  form.appendChild(el('div', { class: 'row' }, [
    el('span', { class: 'hint', text: 'replying as human' }),
    submit,
  ]));
  return form;
}

function buildActions(item) {
  const actions = el('div', { class: 'actions' });
  if (item.status === 'resolved') {
    const btn = el('button', { class: 'btn primary', text: 're-open' });
    btn.type = 'button';
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      btn.disabled = true;
      try {
        await postJson(
          `/api/tasks/${encodeURIComponent(item.task_id)}/review-threads/${encodeURIComponent(item.thread_id)}/reopen`,
        );
        activeThreadId = item.thread_id;
        await fetchAndRender();
      } catch (err) {
        pendingErrors.set(item.thread_id, err.message || String(err));
        btn.disabled = false;
        render();
      }
    });
    actions.appendChild(btn);
  } else {
    const btn = el('button', { class: 'btn primary', text: 'resolve' });
    btn.type = 'button';
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      btn.disabled = true;
      try {
        await postJson(
          `/api/tasks/${encodeURIComponent(item.task_id)}/review-threads/${encodeURIComponent(item.thread_id)}/resolve`,
        );
        activeThreadId = item.thread_id;
        await fetchAndRender();
      } catch (err) {
        pendingErrors.set(item.thread_id, err.message || String(err));
        btn.disabled = false;
        render();
      }
    });
    actions.appendChild(btn);
  }
  return actions;
}

function buildThreadDetail(item) {
  if (!item) {
    return el('div', { class: 'thread-detail empty' }, [
      el('div', { class: 'empty-state' }, [
        el('div', { class: 'icon', text: '✧' }),
        el('div', { class: 'text', text: 'No review thread selected.' }),
      ]),
    ]);
  }

  const errMsg = pendingErrors.get(item.thread_id);
  const detail = el('div', { class: 'thread-detail' });
  detail.appendChild(el('div', { class: 'thread-detail-head' }, [
    el('div', { class: 'crumb' }, [
      el('span', { class: 'id', text: item.task_id || '-' }),
      el('span', { text: '·' }),
      el('span', { text: 'review thread' }),
      el('span', { text: '·' }),
      el('span', { text: item.thread_id || '-' }),
    ]),
    el('h2', { text: item.task_title || item.task_id || item.thread_id }),
    el('div', { class: 'sub' }, [
      threadStatusBadge(item.status || 'open'),
      el('span', { text: `${item.message_count || 0} messages` }),
      el('span', { text: '·' }),
      el('span', { text: `last reply ${fmtRelative(item.last_activity_at)}` }),
      el('span', { text: '·' }),
      el('span', { text: authorLabel(item.last_author_kind, item.last_author_family) }),
    ]),
    buildActions(item),
  ]));
  if (errMsg) {
    detail.appendChild(el('div', { class: 'action-error', text: errMsg }));
  }
  detail.appendChild(buildMessageList(item.messages));
  detail.appendChild(buildReplyForm(item));
  return detail;
}

function render() {
  const body = $('threads-body');
  const countEl = $('threads-count');
  if (!body) return;
  const items = Array.isArray(lastPayload.items) ? lastPayload.items : [];
  const stats = lastPayload.stats || {};
  refreshBadge();
  renderThreadStats(items, stats);

  if (countEl) {
    const visible = items.length;
    const total = Number.isFinite(Number(stats.total)) ? Number(stats.total) : visible;
    countEl.textContent = visible === total ? `${visible}` : `${visible}/${total}`;
  }

  if (items.length === 0) {
    activeThreadId = null;
    syncNodes(body, [el('div', { class: 'empty-state' }, [
      el('div', { class: 'icon', text: '✧' }),
      el('div', { class: 'text', text: 'No review threads match the current filter.' }),
    ])]);
    return;
  }

  const current = activeItem(items);
  const layout = el('div', { class: 'thread-layout' }, [
    buildThreadList(items),
    buildThreadDetail(current),
  ]);
  layout.dataset.key = 'thread-layout';
  layout.dataset.hash = `${activeThreadId}-${items.length}-${computeUnreadCount(items)}-${items.map((item) => `${item.thread_id}:${item.status}:${item.message_count}:${item.last_activity_at}`).join('|')}-${pendingErrors.size}`;
  syncNodes(body, [layout]);
}

export function fetchAndRender() {
  const sp = new URLSearchParams();
  sp.set('status', statusFilter);
  sp.set('author_kind', authorKindFilter);
  return fetchJson(`/api/review-threads?${sp.toString()}`).then((payload) => {
    lastPayload = payload || { items: [], stats: {} };
    buildFilterControls();
    render();
  });
}

export function initReviewThreads() {
  buildFilterControls();
  render();
}

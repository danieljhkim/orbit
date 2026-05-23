// Cross-task review-thread panel for the dashboard.
//
// Lists threads from /api/review-threads with filters (status, author kind),
// supports reply / resolve / re-open in place, and surfaces an unread badge
// for agent-authored threads that appeared since the last visit to this tab.

import { el, fetchJson, postJson, statusPill, syncNodes } from './common.js';

const $ = (id) => document.getElementById(id);

const SEEN_KEY = 'orbit-dashboard.review-threads.seen-agent-thread-ids';

let lastPayload = { items: [], stats: {} };
let statusFilter = 'open';
let authorKindFilter = 'both';
let pendingThreadId = null;
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

function locationLabel(item) {
  const anchor = item.anchor || {};
  if (anchor.kind === 'inline' && item.path && item.line != null) {
    return `${item.path}:${item.line}`;
  }
  return 'task-level';
}

function authorLabel(kind, family) {
  if (kind === 'agent') {
    return family ? `agent (${family})` : 'agent';
  }
  return 'human';
}

function fmtAbs(iso) {
  if (!iso) return '-';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function buildFilterControls() {
  const wrap = $('threads-filters');
  if (!wrap) return;
  wrap.innerHTML = '';

  const statusGroup = el('div', { class: 'thread-filter-group' });
  statusGroup.appendChild(el('span', { class: 'thread-filter-label', text: 'status' }));
  for (const value of ['open', 'resolved', 'all']) {
    const chip = el('button', { class: 'chip', text: value });
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
    if (value === authorKindFilter) chip.classList.add('active');
    chip.addEventListener('click', () => {
      authorKindFilter = value;
      fetchAndRender().catch((err) => console.error(err));
    });
    authorGroup.appendChild(chip);
  }
  wrap.appendChild(authorGroup);
}

function buildMessageList(messages) {
  const list = el('div', { class: 'review-thread-messages' });
  for (const message of messages || []) {
    const author = authorLabel(message.author_kind, message.agent_family);
    const line = el('div', { class: 'comment-line' }, [
      document.createTextNode(`[${fmtAbs(message.at)}] `),
      el('span', { class: 'author', text: author }),
      document.createTextNode(': '),
      el('span', { class: 'review-thread-body', text: message.body || '' }),
    ]);
    list.appendChild(line);
  }
  return list;
}

function buildReplyForm(item) {
  const form = el('div', { class: 'review-thread-reply' });
  const textarea = el('textarea');
  textarea.placeholder = 'Reply to this thread';
  textarea.rows = 3;
  const controls = el('div', { class: 'actions' });
  const submit = el('button', { class: 'action approve', text: 'reply' });
  submit.addEventListener('click', async (e) => {
    e.stopPropagation();
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
      await fetchAndRender();
    } catch (err) {
      pendingErrors.set(item.thread_id, err.message || String(err));
      submit.disabled = false;
      textarea.disabled = false;
      render();
    }
  });
  controls.appendChild(submit);
  form.appendChild(textarea);
  form.appendChild(controls);
  return form;
}

function buildActions(item) {
  const actions = el('div', { class: 'actions' });
  if (item.status === 'resolved') {
    const btn = el('button', { class: 'action approve', text: 're-open' });
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      btn.disabled = true;
      try {
        await postJson(
          `/api/tasks/${encodeURIComponent(item.task_id)}/review-threads/${encodeURIComponent(item.thread_id)}/reopen`,
        );
        await fetchAndRender();
      } catch (err) {
        pendingErrors.set(item.thread_id, err.message || String(err));
        btn.disabled = false;
        render();
      }
    });
    actions.appendChild(btn);
  } else {
    const btn = el('button', { class: 'action archive', text: 'resolve' });
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      btn.disabled = true;
      try {
        await postJson(
          `/api/tasks/${encodeURIComponent(item.task_id)}/review-threads/${encodeURIComponent(item.thread_id)}/resolve`,
        );
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

function render() {
  const body = $('threads-body');
  const countEl = $('threads-count');
  if (!body) return;
  const items = Array.isArray(lastPayload.items) ? lastPayload.items : [];
  refreshBadge();

  if (countEl) {
    const stats = lastPayload.stats || {};
    const visible = items.length;
    const total = Number.isFinite(Number(stats.total)) ? Number(stats.total) : visible;
    countEl.textContent = visible === total ? `${visible}` : `${visible}/${total}`;
  }

  if (items.length === 0) {
    syncNodes(body, [el('div', { class: 'empty-state' }, [
      el('div', { class: 'icon', text: '✧' }),
      el('div', { class: 'text', text: 'No review threads match the current filter.' }),
    ])]);
    return;
  }

  const frag = document.createDocumentFragment();

  const header = el('div', { class: 'review-thread-row header' }, [
    el('span', { text: 'task' }),
    el('span', { text: 'author' }),
    el('span', { text: 'location' }),
    el('span', { text: 'preview' }),
    el('span', { class: 'review-thread-meta', text: 'msgs' }),
    el('span', { text: 'status' }),
    el('span', { class: 'review-thread-meta', text: 'updated' }),
  ]);
  header.dataset.key = 'review-thread-header';
  header.dataset.hash = 'review-thread-header';
  frag.appendChild(header);

  for (const item of items) {
    const row = el('div', { class: 'review-thread-row', title: item.task_title || item.task_id }, [
      el('span', { class: 'mono', text: item.task_id }),
      el('span', { class: 'mono', text: authorLabel(item.last_author_kind, item.last_author_family) }),
      el('span', { class: 'mono', text: locationLabel(item) }),
      el('span', { class: 'review-thread-preview', text: item.body_preview || '' }),
      el('span', { class: 'review-thread-meta', text: String(item.message_count || 0) }),
      statusPill(item.status || 'open'),
      el('span', { class: 'review-thread-meta', text: fmtAbs(item.last_activity_at) }),
    ]);
    row.dataset.key = `thread-row-${item.thread_id}`;
    row.dataset.hash = `${item.thread_id}-${item.status}-${item.message_count}-${item.last_activity_at}-${pendingThreadId === item.thread_id}`;
    if (pendingThreadId === item.thread_id) row.classList.add('expanded');
    row.addEventListener('click', () => {
      pendingThreadId = pendingThreadId === item.thread_id ? null : item.thread_id;
      render();
    });
    frag.appendChild(row);

    if (pendingThreadId === item.thread_id) {
      const detail = el('div', { class: 'review-thread-detail' });
      detail.appendChild(buildMessageList(item.messages));
      const errMsg = pendingErrors.get(item.thread_id);
      if (errMsg) {
        detail.appendChild(el('div', { class: 'action-error', text: errMsg }));
      }
      detail.appendChild(buildReplyForm(item));
      detail.appendChild(buildActions(item));
      detail.dataset.key = `thread-detail-${item.thread_id}`;
      detail.dataset.hash = `${item.thread_id}-${item.status}-${item.message_count}-${item.last_activity_at}-${errMsg || ''}`;
      detail.addEventListener('click', (e) => e.stopPropagation());
      frag.appendChild(detail);
    }
  }

  syncNodes(body, Array.from(frag.children));
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

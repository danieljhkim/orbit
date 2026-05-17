import { mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.dirname(websiteRoot);
const sourcePath = path.join(repoRoot, '.orbit', 'state', 'scoreboard', 'summary.json');
const duelPath = path.join(repoRoot, '.orbit', 'state', 'scoreboard', 'duel_plan.json');
const docsRoot = path.join(websiteRoot, 'src', 'content', 'docs');
const metricsDir = path.join(docsRoot, 'metrics');
const operationsPath = path.join(metricsDir, 'operations.md');
const scoreboardPath = path.join(metricsDir, 'scoreboard.md');
// Legacy single-page route. Removed so /scoreboard doesn't shadow the new
// /metrics/* pages. Safe to delete repeatedly — `force` ignores ENOENT.
const legacyPath = path.join(docsRoot, 'scoreboard.md');

const HIDDEN_AGENTS = new Set(['human', 'agent', 'system', 'admin']);
const KNOWN_AGENT_FAMILIES = ['claude', 'codex', 'gemini', 'grok'];
const TOP_TOOLS_RENDER_LIMIT = 20;

const summary = await loadSummary(sourcePath);

await mkdir(metricsDir, { recursive: true });
await rm(legacyPath, { force: true });

if (!summary) {
  await writeFile(operationsPath, frontmatter('Operations', 'Operation metrics generated from Orbit task history, audit trails, and job runs.') + '# Operations\n\n_No scoreboard summary has been generated yet. Run Orbit in this workspace to populate `.orbit/state/scoreboard/summary.json`._\n');
  await writeFile(scoreboardPath, frontmatter('Scoreboard', 'Per-agent quality and engagement signals.') + '# Scoreboard\n\n_No scoreboard summary has been generated yet. Run Orbit in this workspace to populate `.orbit/state/scoreboard/summary.json`._\n');
  process.exit(0);
}

const agents = Object.entries(summary.agents ?? {})
  .filter(([name]) => !HIDDEN_AGENTS.has(name))
  .sort(([a], [b]) => a.localeCompare(b));
const generatedAt = summary.generated_at ?? null;
const recent = summary.recent_7d ?? null;
const workflowsRun = Array.isArray(summary.workflows_run) ? summary.workflows_run : [];
const topTools = Array.isArray(summary.top_tools) ? summary.top_tools : [];
const duelsByModel = await loadDuelStats(duelPath);

const operationsSections = [
  renderTasksTable(agents, recent),
  renderToolCallsTable(agents, recent),
  renderTopToolsTable(topTools),
  renderWorkflowsTable(workflowsRun, recent),
].filter(Boolean);

const scoreboardSections = [
  renderPrsTable(agents),
  renderDuelsTable(duelsByModel),
  renderTaskReviewTable(agents),
].filter(Boolean);

await writeFile(
  operationsPath,
  frontmatter(
    'Operations',
    'Proof-of-use stats from the Orbit feature surfaces — tasks, tool calls, workflow runs.',
  ) +
    [
      pageHeader(
        'Operations',
        'What agents *do* with Orbit — task lifecycle, tool usage, workflow runs.',
        generatedAt,
        agents.length,
      ),
      ...operationsSections,
    ].join('\n\n') +
    '\n',
);

await writeFile(
  scoreboardPath,
  frontmatter(
    'Scoreboard',
    'Per-agent quality and engagement signals — PRs, planning duels, task reviews.',
  ) +
    [
      pageHeader(
        'Scoreboard',
        'Per-agent quality and engagement signals.',
        generatedAt,
        agents.length,
      ),
      ...scoreboardSections,
    ].join('\n\n') +
    '\n',
);

function frontmatter(title, description) {
  return [
    '---',
    `title: "${title}"`,
    `description: "${description}"`,
    'tableOfContents: false',
    '---',
    '',
  ].join('\n');
}

async function loadSummary(filePath) {
  try {
    const raw = await readFile(filePath, 'utf8');
    return JSON.parse(raw);
  } catch (err) {
    if (err.code === 'ENOENT') return null;
    throw err;
  }
}

async function loadDuelStats(filePath) {
  let runs = [];
  try {
    const raw = await readFile(filePath, 'utf8');
    runs = JSON.parse(raw)?.runs ?? [];
  } catch (err) {
    if (err.code !== 'ENOENT') throw err;
  }

  const stats = new Map(
    KNOWN_AGENT_FAMILIES.map((family) => [
      family,
      { wins: 0, losses: 0, plannerRuns: 0, arbiterRuns: 0 },
    ]),
  );
  const bump = (agent, key) => {
    if (!agent || HIDDEN_AGENTS.has(agent)) return;
    if (!stats.has(agent)) {
      stats.set(agent, { wins: 0, losses: 0, plannerRuns: 0, arbiterRuns: 0 });
    }
    stats.get(agent)[key] += 1;
  };

  for (const run of runs) {
    const a = run.roles?.planner_a?.agent ?? run.roles?.planner_a?.model;
    const b = run.roles?.planner_b?.agent ?? run.roles?.planner_b?.model;
    const arb = run.roles?.arbiter?.agent ?? run.roles?.arbiter?.model;
    const winner = run.outcome?.winner;

    if (a) bump(a, 'plannerRuns');
    if (b) bump(b, 'plannerRuns');
    if (arb) bump(arb, 'arbiterRuns');

    if (winner === 'planner_a') {
      if (a) bump(a, 'wins');
      if (b) bump(b, 'losses');
    } else if (winner === 'planner_b') {
      if (b) bump(b, 'wins');
      if (a) bump(a, 'losses');
    }
  }

  return [...stats.entries()].sort(([a], [b]) => a.localeCompare(b));
}

function pageHeader(title, blurb, generatedAt, agentCount) {
  const lines = [`# ${title}`, '', blurb, ''];
  if (generatedAt) {
    lines.push(`_Generated ${formatTimestamp(generatedAt)} · ${agentCount} agent${agentCount === 1 ? '' : 's'}._`);
  } else if (agentCount > 0) {
    lines.push(`_${agentCount} agent${agentCount === 1 ? '' : 's'}._`);
  }
  return lines.join('\n');
}

function renderTasksTable(agents, recent) {
  const rows = agents
    .filter(([, a]) => (a.tasks_created ?? 0) + (a.tasks_planned ?? 0) + (a.tasks_completed ?? 0) > 0)
    .map(([name, a]) => ({
      sortKey: a.tasks_completed ?? 0,
      cells: [
        agentCell(name),
        num(a.tasks_created ?? 0),
        num(a.tasks_planned ?? 0),
        num(a.tasks_completed ?? 0),
      ],
    }));
  const totalCreated = agents.reduce((sum, [, a]) => sum + (a.tasks_created ?? 0), 0);
  const totalCompleted = agents.reduce((sum, [, a]) => sum + (a.tasks_completed ?? 0), 0);
  const headline = `**${num(totalCompleted)} tasks completed** · ${num(totalCreated)} created${recencyDelta(recent, 'tasks_completed', 'completed this week')}`;
  return section(
    'Tasks',
    'Tasks attributed to each agent, all-time. `Created` and `Planned` count every status; `Completed` counts only `done` and `archived`. Sorted by completed.',
    headline,
    ['Agent', 'Created', 'Planned', 'Completed'],
    sortRows(rows),
  );
}

function renderToolCallsTable(agents, recent) {
  const rows = agents
    .map(([name, a]) => {
      const surfaces = a.tool_calls_by_surface ?? {};
      const graph = surfaces.graph ?? 0;
      const task = surfaces.task ?? 0;
      const total = Object.values(surfaces).reduce((s, v) => s + (v ?? 0), 0);
      return { name, graph, task, total };
    })
    .filter((r) => r.total > 0)
    .map((r) => ({
      sortKey: r.total,
      cells: [agentCell(r.name), num(r.graph), num(r.task), num(r.total)],
    }));
  const totalAll = agents.reduce(
    (sum, [, a]) =>
      sum + Object.values(a.tool_calls_by_surface ?? {}).reduce((s, v) => s + (v ?? 0), 0),
    0,
  );
  const recentSum = recent?.tool_calls_by_surface
    ? Object.values(recent.tool_calls_by_surface).reduce((s, v) => s + (v ?? 0), 0)
    : null;
  const headline = `**${num(totalAll)} \`orbit.*\` tool calls**${
    recentSum != null && recentSum > 0 ? ` · +${num(recentSum)} this week` : ''
  }`;
  return section(
    'Tool calls',
    'Audit-recorded `orbit.*` tool invocations per agent, broken down by Orbit surface. `Total` sums every surface (graph + task + duel + fs + …). Sorted by total.',
    headline,
    ['Agent', 'Graph', 'Task', 'Total'],
    sortRows(rows),
  );
}

function renderTopToolsTable(topTools) {
  const rows = topTools
    .filter((row) => !HIDDEN_AGENTS.has(row.role))
    .slice(0, TOP_TOOLS_RENDER_LIMIT)
    .map((row) => ({
      sortKey: row.count ?? 0,
      cells: [num(row.count ?? 0), agentCell(row.role), `\`${row.tool_name}\``],
    }));
  if (rows.length === 0) return null;
  return section(
    'Most-called tools',
    `Top ${TOP_TOOLS_RENDER_LIMIT} (agent, tool) pairs from the audit log. Restricted to \`orbit.*\` tools.`,
    null,
    ['Calls', 'Agent', 'Tool'],
    sortRows(rows),
  );
}

function renderWorkflowsTable(workflowsRun, recent) {
  const rows = workflowsRun
    .filter((row) => (row.count ?? 0) > 0)
    .map((row) => ({
      sortKey: row.count ?? 0,
      cells: [`\`${row.job_id}\``, num(row.count ?? 0)],
    }));
  const total = workflowsRun.reduce((sum, row) => sum + (row.count ?? 0), 0);
  const headline = `**${num(total)} workflow runs completed**${
    recent?.workflows_run != null && recent.workflows_run > 0 ? ` · +${num(recent.workflows_run)} this week` : ''
  }`;
  return section(
    'Workflow runs',
    'Successful `orbit run` jobs grouped by job definition. Sorted by completed runs.',
    headline,
    ['Job', 'Runs'],
    sortRows(rows),
  );
}

function renderPrsTable(agents) {
  const rows = agents
    .filter(([, a]) => {
      const pr = a.pr ?? {};
      return (pr.merged_clean ?? 0) + (pr.merged_with_revision ?? 0) > 0;
    })
    .map(([name, a]) => {
      const pr = a.pr ?? {};
      const clean = pr.merged_clean ?? 0;
      const revised = pr.merged_with_revision ?? 0;
      const total = clean + revised;
      const rate = total > 0 ? clean / total : null;
      const cleanRate = rate == null ? '—' : `${Math.round(rate * 100)}%`;
      return {
        sortKey: total,
        cells: [agentCell(name), num(total), num(clean), num(revised), cleanRate],
      };
    });
  if (rows.length === 0) return null;
  const total = rows.reduce((sum, row) => sum + row.sortKey, 0);
  const headline = `**${num(total)} PRs landed via the Orbit ship workflow.**`;
  return section(
    'PRs landed',
    'Pull requests merged through `orbit run ship`. Clean rate = `merged_clean / (merged_clean + merged_with_revision)`. Sorted by total.',
    headline,
    ['Agent', 'Total', 'Clean', 'With revision', 'Clean rate'],
    sortRows(rows),
  );
}

function renderDuelsTable(duelsByModel) {
  const rows = duelsByModel
    .map(([name, d]) => {
      const decided = d.wins + d.losses;
      const rate = decided > 0 ? d.wins / decided : null;
      const winRate = rate == null ? '—' : `${Math.round(rate * 100)}%`;
      return {
        sortKey: rate,
        cells: [
          agentCell(name),
          num(d.wins),
          num(d.losses),
          num(d.plannerRuns),
          num(d.arbiterRuns),
          winRate,
        ],
      };
    });
  return section(
    'Planning duels',
    'Head-to-head planning runs. Wins and losses are recorded only for planner roles; arbiter runs decide outcomes and are listed separately. Win rate is `wins / (wins + losses)`. Sorted by win rate.',
    null,
    ['Agent', 'Wins', 'Losses', 'As planner', 'As arbiter', 'Win rate'],
    sortRows(rows),
  );
}

function renderTaskReviewTable(agents) {
  const rows = agents
    .filter(([, a]) => (a.task_review?.threads ?? 0) > 0)
    .map(([name, a]) => ({
      sortKey: a.task_review?.threads ?? 0,
      cells: [agentCell(name), num(a.task_review?.threads ?? 0)],
    }));
  return section(
    'Task review threads',
    'Review threads opened on tasks, attributed to the reviewing model. Sorted by thread count.',
    null,
    ['Agent', 'Threads'],
    sortRows(rows),
  );
}

function recencyDelta(recent, field, label) {
  if (!recent) return '';
  const value = recent[field];
  if (value == null || value === 0) return '';
  return ` · +${num(value)} ${label}`;
}

function sortRows(rows) {
  return [...rows]
    .sort((a, b) => {
      const aNull = a.sortKey == null;
      const bNull = b.sortKey == null;
      if (aNull && bNull) return a.cells[0].localeCompare(b.cells[0]);
      if (aNull) return 1;
      if (bNull) return -1;
      if (b.sortKey !== a.sortKey) return b.sortKey - a.sortKey;
      return a.cells[0].localeCompare(b.cells[0]);
    })
    .map((row) => row.cells);
}

// Now H2 instead of H3 — each metric becomes a top-level section on its
// dedicated page.
function section(title, description, headline, headers, rows) {
  if (rows == null) return null;
  const body = rows.length === 0 ? '_No data yet._' : markdownTable(headers, rows);
  const lines = [`## ${title}`, '', description];
  if (headline) lines.push('', headline);
  lines.push('', body);
  return lines.join('\n');
}

function markdownTable(headers, rows) {
  const headerLine = `| ${headers.join(' | ')} |`;
  const separator = `| ${headers.map(() => '---').join(' | ')} |`;
  const body = rows.map((row) => `| ${row.join(' | ')} |`).join('\n');
  return [headerLine, separator, body].join('\n');
}

function agentCell(name) {
  return `\`${name}\``;
}

function num(value) {
  if (value == null) return '0';
  return Number(value).toLocaleString('en-US');
}

function formatTimestamp(iso) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC');
}

import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.dirname(websiteRoot);
const sourcePath = path.join(repoRoot, '.orbit', 'state', 'scoreboard', 'summary.json');
const duelPath = path.join(repoRoot, '.orbit', 'state', 'scoreboard', 'duel_plan.json');
const targetPath = path.join(websiteRoot, 'src', 'content', 'docs', 'scoreboard.md');

const FRONTMATTER = [
  '---',
  'title: "Scoreboard"',
  'description: "Per-agent metrics for tasks completed, friction reports, planning duels, task review threads, and tool calls."',
  'tableOfContents: false',
  '---',
  '',
].join('\n');

const summary = await loadSummary(sourcePath);

if (!summary) {
  await writeFile(
    targetPath,
    `${FRONTMATTER}# Scoreboard\n\n_No scoreboard summary has been generated yet. Run Orbit in this workspace to populate \`.orbit/state/scoreboard/summary.json\`._\n`,
  );
  process.exit(0);
}

const HIDDEN_AGENTS = new Set(['human', 'agent', 'system', 'admin']);
const agents = Object.entries(summary.agents ?? {})
  .filter(([name]) => !HIDDEN_AGENTS.has(name))
  .sort(([a], [b]) => a.localeCompare(b));
const generatedAt = summary.generated_at ?? null;
const duelsByModel = await loadDuelStats(duelPath);

const sections = [
  renderIntro(generatedAt, agents.length),
  renderTasksTable(agents),
  renderFrictionTable(agents),
  renderDuelsTable(duelsByModel),
  renderTaskReviewTable(agents),
  renderToolCallsTable(agents),
];

await writeFile(targetPath, FRONTMATTER + sections.join('\n\n') + '\n');

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

  const stats = new Map();
  const bump = (model, key) => {
    if (!model || HIDDEN_AGENTS.has(model)) return;
    if (!stats.has(model)) {
      stats.set(model, { wins: 0, losses: 0, plannerRuns: 0, arbiterRuns: 0 });
    }
    stats.get(model)[key] += 1;
  };

  for (const run of runs) {
    const a = run.roles?.planner_a?.model;
    const b = run.roles?.planner_b?.model;
    const arb = run.roles?.arbiter?.model;
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

function renderIntro(generatedAt, agentCount) {
  const lines = ['# Scoreboard', ''];
  lines.push(
    'Per-agent metrics aggregated from Orbit task history, planning duel runs, and audit trails.',
  );
  lines.push('');
  if (generatedAt) {
    lines.push(`_Generated ${formatTimestamp(generatedAt)} • ${agentCount} agent${agentCount === 1 ? '' : 's'}._`);
  } else {
    lines.push(`_${agentCount} agent${agentCount === 1 ? '' : 's'}._`);
  }
  return lines.join('\n');
}

function renderTasksTable(agents) {
  const rows = agents
    .filter(([, a]) => (a.tasks_completed ?? 0) > 0)
    .map(([name, a]) => ({
      sortKey: a.tasks_completed ?? 0,
      cells: [agentCell(name), num(a.tasks_completed)],
    }));
  return section(
    'Tasks completed',
    'Tasks reaching `done` or `archived` status, attributed to the implementing model. Sorted by completed count.',
    ['Agent', 'Completed'],
    sortRows(rows),
  );
}

function renderFrictionTable(agents) {
  const rows = agents
    .filter(([, a]) => {
      const f = a.friction ?? {};
      return (f.reported ?? 0) + (f.accepted ?? 0) + (f.rejected ?? 0) > 0;
    })
    .map(([name, a]) => {
      const f = a.friction ?? {};
      const reported = f.reported ?? 0;
      const accepted = f.accepted ?? 0;
      const rejected = f.rejected ?? 0;
      const rate = reported > 0 ? accepted / reported : null;
      const acceptRate = rate == null ? '—' : `${Math.round(rate * 100)}%`;
      return {
        sortKey: rate,
        cells: [agentCell(name), num(reported), num(accepted), num(rejected), acceptRate],
      };
    });
  return section(
    'Friction bounty',
    'Self-reported agent friction reports. Accept rate = `accepted / reported`. Sorted by accept rate.',
    ['Agent', 'Reported', 'Accepted', 'Rejected', 'Accept rate'],
    sortRows(rows),
  );
}

function renderDuelsTable(duelsByModel) {
  const rows = duelsByModel
    .filter(([, d]) => d.wins + d.losses + d.plannerRuns + d.arbiterRuns > 0)
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
    ['Agent', 'Threads'],
    sortRows(rows),
  );
}

function renderToolCallsTable(agents) {
  const rows = agents
    .filter(([, a]) => (a.tool_calls ?? 0) + (a.failed_tool_calls ?? 0) > 0)
    .map(([name, a]) => {
      const total = a.tool_calls ?? 0;
      const failed = a.failed_tool_calls ?? 0;
      const rate = total > 0 ? failed / total : null;
      const failRate = rate == null ? '—' : `${Math.round(rate * 100)}%`;
      return {
        sortKey: rate,
        cells: [agentCell(name), num(total), num(failed), failRate],
      };
    });
  return section(
    'Tool calls',
    'Tool invocations recorded in the audit trail. Failure rate = `failed / total`. Sorted by failure rate (highest first).',
    ['Agent', 'Total', 'Failed', 'Failure rate'],
    sortRows(rows),
  );
}

function sortRows(rows) {
  // Sort descending by sortKey; nulls (undefined rates) go last; stable agent-name tiebreaker.
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

function section(title, description, headers, rows) {
  if (rows.length === 0) {
    return `## ${title}\n\n${description}\n\n_No data yet._`;
  }
  return `## ${title}\n\n${description}\n\n${markdownTable(headers, rows)}`;
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

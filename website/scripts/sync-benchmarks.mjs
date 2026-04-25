import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.dirname(websiteRoot);
const sourceRoot = path.join(repoRoot, 'benchmarks', 'graph');
const targetRoot = path.join(websiteRoot, 'src', 'content', 'docs', 'benchmarks', 'graph');
const editBaseUrl = 'https://github.com/danieljhkim/orbit/edit/main/';

const orderByFile = new Map([
  ['README.md', 1],
  ['RESULTS.md', 10],
  ['METHOD.md', 20],
  ['V2_FIXTURES.md', 30],
  ['ISSUES.md', 40],
]);

const labelByFile = new Map([
  ['RESULTS.md', 'Results'],
  ['METHOD.md', 'Methodology'],
  ['V2_FIXTURES.md', 'Fixtures'],
  ['ISSUES.md', 'Issues'],
]);

await rm(targetRoot, { recursive: true, force: true });
await mkdir(targetRoot, { recursive: true });

for (const file of await collectMarkdown(sourceRoot)) {
  const relative = path.relative(sourceRoot, file);
  const target = path.join(targetRoot, relative);
  const raw = await readFile(file, 'utf8');
  const { title: rawTitle, body } = splitTitle(raw, relative);
  const title = normalizeTitle(rawTitle, relative);
  const pageBody = rewriteMarkdownLinks(body, relative);
  const description = summarize(pageBody, title);

  const sourcePath = path.posix.join('benchmarks/graph', toPosix(relative));
  const order = orderByFile.get(path.basename(relative));

  const sidebarLabel = labelByFile.get(path.basename(relative));

  const frontmatter = [
    '---',
    `title: ${JSON.stringify(title)}`,
    `description: ${JSON.stringify(description)}`,
    `editUrl: ${JSON.stringify(editBaseUrl + sourcePath)}`,
    ...(order || sidebarLabel ? [
      `sidebar:`,
      ...(sidebarLabel ? [`  label: ${JSON.stringify(sidebarLabel)}`] : []),
      ...(order ? [`  order: ${order}`] : [])
    ] : []),
    '---',
    '',
  ].join('\n');
  const notice = [
    `> Mirrored from \`${sourcePath}\`. Edit the source document in the repository, not this generated page.`,
    '',
    '<style>',
    '  :root {',
    '    --sl-content-width: 72rem;',
    '  }',
    '</style>',
    '',
    '',
  ].join('\n');

  const finalTarget = path.basename(target).toLowerCase() === 'readme.md' 
    ? path.join(path.dirname(target), 'index.md') 
    : target;

  await mkdir(path.dirname(finalTarget), { recursive: true });
  await writeFile(finalTarget, `${frontmatter}${notice}${pageBody.trim()}\n`, 'utf8');
}

async function collectMarkdown(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.name.startsWith('_') || entry.name.startsWith('.')) continue; // skip _archive, .git etc.
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectMarkdown(absolute)));
    } else if (entry.isFile() && entry.name.endsWith('.md')) {
      files.push(absolute);
    }
  }
  return files.sort((a, b) => a.localeCompare(b));
}

function splitTitle(raw, relative) {
  const lines = raw.replace(/\r\n/g, '\n').split('\n');
  const firstHeading = lines.findIndex((line) => line.startsWith('# '));
  if (firstHeading === -1) {
    return { title: titleFromPath(relative), body: raw };
  }
  const title = lines[firstHeading].replace(/^#\s+/, '').trim();
  const body = lines.slice(firstHeading + 1).join('\n').trimStart();
  return { title, body };
}

function rewriteMarkdownLinks(raw, currentRelative) {
  return raw.replace(/\[([^\]]+)\]\(([^)]+\.md(?:#[^)]+)?)\)/g, (match, label, href) => {
    if (/^[a-z]+:/i.test(href) || href.startsWith('/')) return match;
    const [withoutHash, hash = ''] = href.split('#');
    
    const sourceTarget = path.posix.normalize(
      path.posix.join(path.posix.dirname(toPosix(currentRelative)), withoutHash)
    );
    if (sourceTarget.startsWith('..')) return match;
    const currentRoute = routeDirFor(currentRelative);
    const targetRoute = routeDirFor(sourceTarget);
    let relativeRoute = path.posix.relative(currentRoute, targetRoute);
    if (!relativeRoute) relativeRoute = '.';
    if (!relativeRoute.endsWith('/')) relativeRoute += '/';
    return `[${label}](${relativeRoute}${hash ? `#${hash}` : ''})`;
  });
}

function routeDirFor(relative) {
  let withoutExt = toPosix(relative).replace(/\.md$/i, '').toLowerCase();
  if (withoutExt === 'readme' || withoutExt.endsWith('/readme')) {
    withoutExt = withoutExt.replace(/\/?readme$/, '');
  }
  return path.posix.join('benchmarks/graph', withoutExt, '/');
}

function summarize(body, title) {
  const paragraph = body
    .split(/\n\s*\n/)
    .map((block) => block.trim())
    .find((block) => block && !block.startsWith('```') && !block.startsWith('|') && !block.startsWith('>') && !block.startsWith('#') && !block.startsWith('-'));
  if (!paragraph) return `${title} benchmark results.`;
  const flattened = paragraph
    .replace(/\s+/g, ' ')
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .trim();
  if (flattened.length <= 156) return flattened;
  return flattened.slice(0, 156).replace(/\s+\S*$/, '').trim() + '…';
}

function normalizeTitle(title, relative) {
  let cleaned = title.replace(/^graph\//, '').replace(/\s*[—-]\s*[A-Z]{2,}\s*$/, '').trim();
  if (path.basename(relative).toLowerCase() === 'readme.md') {
    const dir = path.posix.dirname(toPosix(relative));
    if (dir !== '.' && /^v\d+$/.test(path.posix.basename(dir))) {
      cleaned = path.posix.basename(dir);
    }
  }
  return cleaned || titleFromPath(relative);
}

function titleFromPath(relative) {
  return path
    .basename(relative, '.md')
    .replace(/^\d+_/, '')
    .replace(/[-_]/g, ' ')
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function toPosix(value) {
  return value.split(path.sep).join('/');
}

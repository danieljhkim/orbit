import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.dirname(websiteRoot);
const sourceRoot = path.join(repoRoot, 'docs', 'design');
const targetRoot = path.join(websiteRoot, 'src', 'content', 'docs', 'architecture', 'design');
const editBaseUrl = 'https://github.com/danieljhkim/orbit/edit/main/';

const orderByFile = new Map([
  ['CONVENTIONS.md', 1],
  ['1_overview.md', 10],
  ['2_design.md', 20],
  ['3_vision.md', 30],
  ['4_decisions.md', 40],
  ['5_null_result.md', 50],
  ['README.md', 5],
]);

await rm(targetRoot, { recursive: true, force: true });
await mkdir(targetRoot, { recursive: true });

for (const file of await collectMarkdown(sourceRoot)) {
  const relative = path.relative(sourceRoot, file);
  const target = path.join(targetRoot, relative);
  const raw = await readFile(file, 'utf8');
  const { title, body } = splitTitle(raw, relative);
  const pageBody = rewriteMarkdownLinks(stripDesignMetadata(body), relative);
  const description = summarize(pageBody, title);
  const order = orderByFile.get(path.basename(relative));
  const sidebarLabel = sidebarLabelFor(relative);
  const sourcePath = path.posix.join('docs/design', toPosix(relative));
  const sidebarLines = [];
  if (sidebarLabel || order) sidebarLines.push('sidebar:');
  if (sidebarLabel) sidebarLines.push(`  label: ${JSON.stringify(sidebarLabel)}`);
  if (order) sidebarLines.push(`  order: ${order}`);
  const frontmatter = [
    '---',
    `title: ${JSON.stringify(title)}`,
    `description: ${JSON.stringify(description)}`,
    `editUrl: ${JSON.stringify(editBaseUrl + sourcePath)}`,
    ...sidebarLines,
    '---',
    '',
  ].join('\n');
  const notice = [
    `> Mirrored from \`${sourcePath}\`. Edit the source document in the repository, not this generated page.`,
    '',
    '',
  ].join('\n');

  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, `${frontmatter}${notice}${pageBody.trim()}\n`, 'utf8');
}

async function collectMarkdown(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
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

function stripDesignMetadata(raw) {
  const lines = raw.replace(/\r\n/g, '\n').split('\n');
  while (lines.length && lines[0].trim() === '') lines.shift();
  while (
    lines.length &&
    /^(\*\*)?(Status|Owner|Last updated):/i.test(lines[0].trim())
  ) {
    lines.shift();
  }
  while (lines.length && lines[0].trim() === '') lines.shift();
  if (lines[0]?.trim() === '---') lines.shift();
  while (lines.length && lines[0].trim() === '') lines.shift();
  return lines.join('\n');
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
  const withoutExt = toPosix(relative).replace(/\.md$/i, '').toLowerCase();
  return path.posix.join('architecture/design', withoutExt, '/');
}

function summarize(body, title) {
  const paragraph = body
    .split(/\n\s*\n/)
    .map((block) => block.replace(/\*\*Status:\*\*.*\n?/g, '').replace(/\*\*Owner:\*\*.*\n?/g, '').replace(/\*\*Last updated:\*\*.*\n?/g, '').trim())
    .find((block) => block && !block.startsWith('```') && !block.startsWith('|') && !block.startsWith('>'));
  if (!paragraph) return `${title} from the Orbit architecture design docs.`;
  return paragraph
    .replace(/\s+/g, ' ')
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .slice(0, 156)
    .replace(/\s+\S*$/, '')
    .trim();
}

function sidebarLabelFor(relative) {
  const base = path.basename(relative, '.md');
  if (base.toLowerCase() === 'readme') return null;
  const stripped = base.replace(/^\d+_/, '');
  if (!stripped) return null;
  return stripped
    .replace(/[-_]/g, ' ')
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
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

import { rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const targetRoot = path.join(
  websiteRoot,
  'src',
  'content',
  'docs',
  'architecture',
  'design',
);

// Design records contain implementation history, internal artifact identifiers,
// and retired interfaces. They remain source-controlled under docs/design/, but
// are intentionally not copied into the public, current-surface documentation.
await rm(targetRoot, { recursive: true, force: true });

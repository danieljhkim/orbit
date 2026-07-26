import { rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const targetRoot = path.join(
  websiteRoot,
  'src',
  'content',
  'docs',
  'benchmarks',
  'graph',
);

// The graph benchmark corpus exercises a retired command surface. Keep the
// source benchmark records in-repo without publishing them as current usage.
await rm(targetRoot, { recursive: true, force: true });

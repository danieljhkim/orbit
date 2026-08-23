#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');

const PKG_ROOT = path.resolve(__dirname, '..');
const DEFAULT_BIN = path.join(
  PKG_ROOT,
  'binaries',
  process.platform === 'win32' ? 'orbit.exe' : 'orbit'
);

function resolveBinary() {
  if (process.env.ORBIT_BINARY) return process.env.ORBIT_BINARY;
  if (fs.existsSync(DEFAULT_BIN)) return DEFAULT_BIN;
  // Lazy install path: handles `npm install --ignore-scripts`.
  process.stderr.write('@orbit-tools/cli: binary not found, attempting download...\n');
  const installer = path.join(PKG_ROOT, 'scripts', 'install-binary.js');
  const result = spawnSync(process.execPath, [installer], { stdio: 'inherit' });
  if (result.status !== 0 || !fs.existsSync(DEFAULT_BIN)) {
    process.stderr.write('@orbit-tools/cli: binary install failed. Set ORBIT_BINARY to point at a local orbit binary, or reinstall the package.\n');
    process.exit(result.status || 1);
  }
  return DEFAULT_BIN;
}

const binary = resolveBinary();
const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code === null ? 1 : code);
  }
});

child.on('error', (err) => {
  process.stderr.write(`@orbit-tools/cli: failed to launch ${binary}: ${err.message}\n`);
  process.exit(1);
});

#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const os = require('node:os');

const PKG = require('../package.json');
const BINARY_REPO = PKG.config.orbit.binaryRepo;
// Convention: npm package version is kept in lockstep with the orbit release tag.
// `0.3.1` → fetches `v0.3.1` from GitHub Releases. Override with $ORBIT_BINARY_VERSION.
const BINARY_VERSION = process.env.ORBIT_BINARY_VERSION || `v${PKG.version}`;
const PKG_ROOT = path.resolve(__dirname, '..');
const BIN_DIR = path.join(PKG_ROOT, 'binaries');
const BIN_PATH = path.join(BIN_DIR, process.platform === 'win32' ? 'orbit.exe' : 'orbit');

function log(msg) {
  process.stderr.write(`@orbit-tools/cli: ${msg}\n`);
}

function fail(msg) {
  process.stderr.write(`@orbit-tools/cli: ${msg}\n`);
  process.exit(1);
}

function resolveTarget() {
  const platform = process.platform;
  const arch = process.arch;
  const key = `${platform}-${arch}`;
  const map = {
    'darwin-arm64': 'aarch64-apple-darwin',
    'darwin-x64': 'x86_64-apple-darwin',
    'linux-x64': 'x86_64-unknown-linux-gnu',
    'linux-arm64': 'aarch64-unknown-linux-gnu',
  };
  const target = map[key];
  if (!target) {
    fail(`unsupported platform/arch: ${key}. Supported: ${Object.keys(map).join(', ')}`);
  }
  return target;
}

function fetchBuffer(url, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { 'user-agent': '@orbit-tools/cli installer' } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          if (redirectsLeft <= 0) return reject(new Error(`too many redirects fetching ${url}`));
          res.resume();
          return resolve(fetchBuffer(res.headers.location, redirectsLeft - 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode} fetching ${url}`));
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks)));
        res.on('error', reject);
      })
      .on('error', reject);
  });
}

function sha256(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex');
}

function parseChecksums(text) {
  const out = {};
  for (const line of text.split('\n')) {
    const m = line.trim().match(/^([a-f0-9]{64})\s+(\S+)$/i);
    if (m) out[m[2]] = m[1].toLowerCase();
  }
  return out;
}

function extractTarGz(archivePath, destDir) {
  const result = spawnSync('tar', ['-xzf', archivePath, '-C', destDir], { stdio: 'inherit' });
  if (result.status !== 0) {
    fail(`tar extraction failed (status ${result.status}). Is 'tar' installed?`);
  }
}

async function main() {
  if (process.env.ORBIT_SKIP_DOWNLOAD === '1') {
    log('ORBIT_SKIP_DOWNLOAD=1 set; skipping binary download.');
    return;
  }
  if (process.env.ORBIT_BINARY) {
    log(`ORBIT_BINARY=${process.env.ORBIT_BINARY} set; skipping download (bin shim will use it directly).`);
    return;
  }

  const target = resolveTarget();
  const asset = `orbit-${target}.tar.gz`;
  const baseUrl = `https://github.com/${BINARY_REPO}/releases/download/${BINARY_VERSION}`;
  const archiveUrl = `${baseUrl}/${asset}`;
  const checksumUrl = `${baseUrl}/orbit-checksums.txt`;

  log(`installing orbit ${BINARY_VERSION} for ${target}...`);

  fs.mkdirSync(BIN_DIR, { recursive: true });

  let archiveBuf;
  try {
    archiveBuf = await fetchBuffer(archiveUrl);
  } catch (err) {
    fail(`failed to download ${archiveUrl}: ${err.message}`);
  }

  try {
    const checksumText = (await fetchBuffer(checksumUrl)).toString('utf8');
    const checksums = parseChecksums(checksumText);
    const expected = checksums[asset];
    if (!expected) {
      fail(`checksum entry for ${asset} was not found in orbit-checksums.txt`);
    } else {
      const actual = sha256(archiveBuf);
      if (actual !== expected) {
        fail(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
      }
    }
  } catch (err) {
    fail(`could not verify checksum for ${asset}: ${err.message}`);
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'orbit-cli-'));
  const archivePath = path.join(tmpDir, asset);
  fs.writeFileSync(archivePath, archiveBuf);
  extractTarGz(archivePath, tmpDir);

  const extractedBinary = path.join(tmpDir, 'orbit');
  if (!fs.existsSync(extractedBinary)) {
    fail(`extracted archive did not contain 'orbit' binary at ${extractedBinary}`);
  }
  fs.copyFileSync(extractedBinary, BIN_PATH);
  fs.chmodSync(BIN_PATH, 0o755);
  fs.rmSync(tmpDir, { recursive: true, force: true });

  log(`installed orbit binary at ${BIN_PATH}`);
}

main().catch((err) => fail(err && err.message ? err.message : String(err)));

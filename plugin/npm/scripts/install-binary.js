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
const CHECKSUM_FILE = 'orbit-checksums.txt';
const CHECKSUM_SIGNATURE_FILE = 'orbit-checksums.txt.sig';
// Convention: npm package version is kept in lockstep with the orbit release tag.
// `0.3.1` → fetches `v0.3.1` from GitHub Releases. Override with $ORBIT_BINARY_VERSION.
const BINARY_VERSION = process.env.ORBIT_BINARY_VERSION || `v${PKG.version}`;
const PKG_ROOT = path.resolve(__dirname, '..');
const BIN_DIR = path.join(PKG_ROOT, 'binaries');
const BIN_PATH = path.join(BIN_DIR, process.platform === 'win32' ? 'orbit.exe' : 'orbit');
const TRUSTED_PUBLIC_KEY_PATH = process.env.ORBIT_RELEASE_PUBLIC_KEY_FILE
  ? path.resolve(process.env.ORBIT_RELEASE_PUBLIC_KEY_FILE)
  : path.join(PKG_ROOT, 'release-signing.pub');

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

function readTrustedPublicKey() {
  return fs.readFileSync(TRUSTED_PUBLIC_KEY_PATH, 'utf8');
}

function verifyChecksumSignature(checksumText, signatureBuf, publicKeyPem = readTrustedPublicKey()) {
  const verifier = crypto.createVerify('RSA-SHA256');
  verifier.update(checksumText, 'utf8');
  verifier.end();
  if (!verifier.verify(publicKeyPem, signatureBuf)) {
    throw new Error(`release checksum signature verification failed for ${CHECKSUM_FILE}`);
  }
}

function verifyArchiveChecksum(asset, archiveBuf, checksumText) {
  const checksums = parseChecksums(checksumText);
  const expected = checksums[asset];
  if (!expected) {
    throw new Error(`checksum entry for ${asset} was not found in ${CHECKSUM_FILE}`);
  }
  const actual = sha256(archiveBuf);
  if (actual !== expected) {
    throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
  }
}

function runTar(args, options = {}) {
  const result = spawnSync('tar', args, { encoding: 'utf8', ...options });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = result.stderr ? `: ${result.stderr.trim()}` : '';
    throw new Error(`tar ${args.join(' ')} failed (status ${result.status})${stderr}`);
  }
  return result.stdout || '';
}

function validateArchiveMemberName(member) {
  if (member === 'orbit') {
    return;
  }
  if (!member || member.startsWith('/') || member.includes('..') || path.isAbsolute(member)) {
    throw new Error(`unsafe release archive member: ${member || '<empty>'}`);
  }
  throw new Error(`unexpected release archive member: ${member}`);
}

function validateArchiveMembers(archivePath) {
  const members = runTar(['-tzf', archivePath])
    .split(/\r?\n/)
    .filter((line) => line.length > 0);
  if (members.length !== 1) {
    throw new Error("release archive must contain only the 'orbit' binary");
  }
  validateArchiveMemberName(members[0]);

  const verbose = runTar(['-tvzf', archivePath])
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  if (verbose.length !== 1) {
    throw new Error("release archive must contain exactly one 'orbit' member");
  }
  if (!verbose[0].startsWith('-')) {
    throw new Error(`release archive member must be a regular file: ${members[0]}`);
  }
}

function extractTarGz(archivePath, destDir) {
  validateArchiveMembers(archivePath);
  runTar(['-xzf', archivePath, '-C', destDir, 'orbit'], { stdio: 'inherit' });
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
  const checksumUrl = `${baseUrl}/${CHECKSUM_FILE}`;
  const checksumSignatureUrl = `${baseUrl}/${CHECKSUM_SIGNATURE_FILE}`;

  log(`installing orbit ${BINARY_VERSION} for ${target}...`);

  fs.mkdirSync(BIN_DIR, { recursive: true });

  let checksumText;
  let signatureBuf;
  try {
    checksumText = (await fetchBuffer(checksumUrl)).toString('utf8');
    signatureBuf = await fetchBuffer(checksumSignatureUrl);
    verifyChecksumSignature(checksumText, signatureBuf);
  } catch (err) {
    fail(`could not authenticate ${CHECKSUM_FILE}: ${err.message}`);
  }

  let archiveBuf;
  try {
    archiveBuf = await fetchBuffer(archiveUrl);
  } catch (err) {
    fail(`failed to download ${archiveUrl}: ${err.message}`);
  }

  try {
    verifyArchiveChecksum(asset, archiveBuf, checksumText);
  } catch (err) {
    fail(err.message);
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'orbit-cli-'));
  try {
    const archivePath = path.join(tmpDir, asset);
    fs.writeFileSync(archivePath, archiveBuf);
    extractTarGz(archivePath, tmpDir);

    const extractedBinary = path.join(tmpDir, 'orbit');
    if (!fs.existsSync(extractedBinary)) {
      fail(`extracted archive did not contain 'orbit' binary at ${extractedBinary}`);
    }
    fs.copyFileSync(extractedBinary, BIN_PATH);
    fs.chmodSync(BIN_PATH, 0o755);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }

  log(`installed orbit binary at ${BIN_PATH}`);
}

if (require.main === module) {
  main().catch((err) => fail(err && err.message ? err.message : String(err)));
}

module.exports = {
  parseChecksums,
  sha256,
  validateArchiveMembers,
  verifyArchiveChecksum,
  verifyChecksumSignature,
};

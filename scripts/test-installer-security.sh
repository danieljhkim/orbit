#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

require_bin() {
  local bin="$1"
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "test-installer-security: required binary '$bin' not on PATH" >&2
    exit 2
  fi
}

require_bin node
require_bin openssl
require_bin tar

target_name() {
  local os arch
  os="$(uname -s 2>/dev/null || true)"
  arch="$(uname -m 2>/dev/null || true)"

  case "${os}/${arch}" in
    Darwin/arm64 | Darwin/aarch64) echo "aarch64-apple-darwin" ;;
    Darwin/x86_64 | Darwin/amd64) echo "x86_64-apple-darwin" ;;
    Linux/x86_64 | Linux/amd64) echo "x86_64-unknown-linux-gnu" ;;
    Linux/arm64 | Linux/aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *)
      echo "test-installer-security: unsupported platform ${os:-unknown}/${arch:-unknown}" >&2
      exit 2
      ;;
  esac
}

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    openssl dgst -sha256 "$file" | awk '{print $NF}'
  fi
}

write_test_binary() {
  local dir="$1"
  cat > "$dir/orbit" <<'BIN'
#!/bin/sh
if [ -n "${ORBIT_TEST_EXEC_MARKER:-}" ]; then
  printf '%s\n' executed > "$ORBIT_TEST_EXEC_MARKER"
fi
printf '%s\n' "orbit test 0.0.0"
BIN
  chmod 755 "$dir/orbit"
}

sign_checksums() {
  local checksums="$1"
  local signature="$2"
  openssl dgst -sha256 -sign "$PRIVATE_KEY" -out "$signature" "$checksums"
}

make_regular_archive() {
  local archive="$1"
  local build_dir="$TMP_ROOT/build-regular"
  rm -rf "$build_dir"
  mkdir -p "$build_dir"
  write_test_binary "$build_dir"
  tar -czf "$archive" -C "$build_dir" orbit
}

make_symlink_archive() {
  local archive="$1"
  local build_dir="$TMP_ROOT/build-symlink"
  rm -rf "$build_dir"
  mkdir -p "$build_dir"
  ln -s /etc/passwd "$build_dir/orbit"
  tar -czf "$archive" -C "$build_dir" orbit
}

make_traversal_archive() {
  local archive="$1"
  local build_dir="$TMP_ROOT/build-traversal"
  rm -rf "$build_dir"
  mkdir -p "$build_dir/nested"
  printf '%s\n' "not orbit" > "$build_dir/evil"
  tar -czf "$archive" -C "$build_dir/nested" ../evil
}

prepare_release_dir() {
  local name="$1"
  local archive_kind="$2"
  local checksum_kind="$3"
  local release_dir="$TMP_ROOT/$name"
  local asset="orbit-${TARGET}.tar.gz"
  local archive="$release_dir/$asset"
  mkdir -p "$release_dir"

  case "$archive_kind" in
    regular) make_regular_archive "$archive" ;;
    symlink) make_symlink_archive "$archive" ;;
    traversal) make_traversal_archive "$archive" ;;
    *)
      echo "unknown archive kind: $archive_kind" >&2
      exit 2
      ;;
  esac

  case "$checksum_kind" in
    correct) printf '%s  %s\n' "$(sha256_file "$archive")" "$asset" > "$release_dir/orbit-checksums.txt" ;;
    wrong) printf '%064d  %s\n' 0 "$asset" > "$release_dir/orbit-checksums.txt" ;;
    *)
      echo "unknown checksum kind: $checksum_kind" >&2
      exit 2
      ;;
  esac

  sign_checksums "$release_dir/orbit-checksums.txt" "$release_dir/orbit-checksums.txt.sig"
  echo "$release_dir"
}

run_shell_install() {
  local release_dir="$1"
  local install_dir="$2"
  local marker="$3"
  ORBIT_INSTALL_BASE_URL="file://$release_dir" \
    ORBIT_INSTALL_DIR="$install_dir" \
    ORBIT_RELEASE_PUBLIC_KEY_FILE="$PUBLIC_KEY" \
    ORBIT_TEST_EXEC_MARKER="$marker" \
    sh "$ROOT/install.sh"
}

expect_shell_failure() {
  local release_dir="$1"
  local label="$2"
  local install_dir="$TMP_ROOT/install-$label"
  local marker="$TMP_ROOT/marker-$label"
  local log_file="$TMP_ROOT/$label.log"

  if run_shell_install "$release_dir" "$install_dir" "$marker" > "$log_file" 2>&1; then
    echo "FAIL: shell installer accepted $label" >&2
    cat "$log_file" >&2
    exit 1
  fi
  if [ -e "$marker" ]; then
    echo "FAIL: shell installer executed binary for $label" >&2
    exit 1
  fi
}

PRIVATE_KEY="$TMP_ROOT/release-signing.key"
PUBLIC_KEY="$TMP_ROOT/release-signing.pub"
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$PRIVATE_KEY" >/dev/null 2>&1
openssl rsa -pubout -in "$PRIVATE_KEY" -out "$PUBLIC_KEY" >/dev/null 2>&1

TARGET="$(target_name)"

good_release="$(prepare_release_dir good regular correct)"
bad_checksum_release="$(prepare_release_dir bad-checksum regular wrong)"
symlink_release="$(prepare_release_dir symlink symlink correct)"
traversal_release="$(prepare_release_dir traversal traversal correct)"

expect_shell_failure "$bad_checksum_release" "checksum-mismatch"
expect_shell_failure "$symlink_release" "symlink-member"
expect_shell_failure "$traversal_release" "traversal-member"

good_install_dir="$TMP_ROOT/install-good"
good_marker="$TMP_ROOT/marker-good"
run_shell_install "$good_release" "$good_install_dir" "$good_marker" > "$TMP_ROOT/good.log" 2>&1
test -x "$good_install_dir/orbit"
test -f "$good_marker"

npm_checksums="$TMP_ROOT/npm-checksums.txt"
npm_signature="$TMP_ROOT/npm-checksums.txt.sig"
good_archive="$good_release/orbit-${TARGET}.tar.gz"
printf '%s  %s\n' "$(sha256_file "$good_archive")" "orbit-${TARGET}.tar.gz" > "$npm_checksums"
sign_checksums "$npm_checksums" "$npm_signature"

ROOT="$ROOT" \
  PUBLIC_KEY="$PUBLIC_KEY" \
  CHECKSUMS="$npm_checksums" \
  SIGNATURE="$npm_signature" \
  GOOD_ARCHIVE="$good_archive" \
  SYMLINK_ARCHIVE="$symlink_release/orbit-${TARGET}.tar.gz" \
  TRAVERSAL_ARCHIVE="$traversal_release/orbit-${TARGET}.tar.gz" \
  TARGET="$TARGET" \
  node <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const installer = require(path.join(process.env.ROOT, 'plugin/npm/scripts/install-binary.js'));

function expectThrow(fn, pattern, label) {
  try {
    fn();
  } catch (err) {
    if (!pattern.test(err.message)) {
      throw new Error(`${label}: expected ${pattern}, got ${err.message}`);
    }
    return;
  }
  throw new Error(`${label}: expected failure`);
}

const publicKey = fs.readFileSync(process.env.PUBLIC_KEY, 'utf8');
const checksumText = fs.readFileSync(process.env.CHECKSUMS, 'utf8');
const signature = fs.readFileSync(process.env.SIGNATURE);
const goodArchive = fs.readFileSync(process.env.GOOD_ARCHIVE);
const asset = `orbit-${process.env.TARGET}.tar.gz`;

installer.verifyChecksumSignature(checksumText, signature, publicKey);
installer.verifyArchiveChecksum(asset, goodArchive, checksumText);
installer.validateArchiveMembers(process.env.GOOD_ARCHIVE);

const tamperedSignature = Buffer.from(signature);
tamperedSignature[0] = tamperedSignature[0] ^ 0xff;
expectThrow(
  () => installer.verifyChecksumSignature(checksumText, tamperedSignature, publicKey),
  /signature verification failed/,
  'npm signature failure'
);
expectThrow(
  () => installer.verifyArchiveChecksum(asset, goodArchive, `0000000000000000000000000000000000000000000000000000000000000000  ${asset}\n`),
  /checksum mismatch/,
  'npm checksum failure'
);
expectThrow(
  () => installer.validateArchiveMembers(process.env.SYMLINK_ARCHIVE),
  /regular file/,
  'npm symlink archive rejection'
);
expectThrow(
  () => installer.validateArchiveMembers(process.env.TRAVERSAL_ARCHIVE),
  /unsafe release archive member/,
  'npm traversal archive rejection'
);
NODE

echo "test-installer-security: ok"

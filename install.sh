#!/bin/sh

set -eu

REPO="${ORBIT_INSTALL_REPO:-danieljhkim/orbit}"
BINARY_NAME="orbit"
CHECKSUM_FILE="orbit-checksums.txt"
INSTALL_DIR="${ORBIT_INSTALL_DIR:-$HOME/.orbit/bin}"

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'orbit installer: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

download() {
  url="$1"
  destination="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$destination"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -qO "$destination" "$url"
    return
  fi

  fail "curl or wget is required to download Orbit releases"
}

compute_sha256() {
  file="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
    return
  fi

  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$file" | awk '{print $NF}'
    return
  fi

  fail "sha256sum, shasum, or openssl is required to verify downloads"
}

normalize_tag() {
  version="$1"

  case "$version" in
    v*)
      printf '%s' "$version"
      ;;
    *)
      printf 'v%s' "$version"
      ;;
  esac
}

resolve_target() {
  os="$(uname -s 2>/dev/null || true)"
  arch="$(uname -m 2>/dev/null || true)"

  case "${os}/${arch}" in
    Darwin/arm64 | Darwin/aarch64)
      printf 'aarch64-apple-darwin'
      ;;
    Darwin/x86_64 | Darwin/amd64)
      printf 'x86_64-apple-darwin'
      ;;
    Linux/x86_64 | Linux/amd64)
      printf 'x86_64-unknown-linux-gnu'
      ;;
    Linux/arm64 | Linux/aarch64)
      printf 'aarch64-unknown-linux-gnu'
      ;;
    *)
      fail "unsupported platform: ${os:-unknown}/${arch:-unknown}"
      ;;
  esac
}

need_cmd awk
need_cmd install
need_cmd mktemp
need_cmd tar

TARGET="$(resolve_target)"
ARCHIVE_NAME="orbit-${TARGET}.tar.gz"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/orbit-install.XXXXXX")"

cleanup() {
  rm -rf "$TMP_DIR"
}

trap cleanup EXIT HUP INT TERM

if [ -n "${ORBIT_VERSION:-}" ]; then
  RELEASE_TAG="$(normalize_tag "$ORBIT_VERSION")"
  BASE_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}"
  VERSION_LABEL="$RELEASE_TAG"
else
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
  VERSION_LABEL="latest"
fi

ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"
CHECKSUM_PATH="${TMP_DIR}/${CHECKSUM_FILE}"

log "Downloading Orbit ${VERSION_LABEL} for ${TARGET}..."
download "${BASE_URL}/${CHECKSUM_FILE}" "$CHECKSUM_PATH"
download "${BASE_URL}/${ARCHIVE_NAME}" "$ARCHIVE_PATH"

EXPECTED_SHA="$(awk -v asset="$ARCHIVE_NAME" '$2 == asset { print $1 }' "$CHECKSUM_PATH")"
[ -n "$EXPECTED_SHA" ] || fail "checksum entry for ${ARCHIVE_NAME} was not found in ${CHECKSUM_FILE}"

ACTUAL_SHA="$(compute_sha256 "$ARCHIVE_PATH")"

if [ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]; then
  fail "checksum verification failed for ${ARCHIVE_NAME} (expected ${EXPECTED_SHA}, got ${ACTUAL_SHA})"
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"
[ -f "${TMP_DIR}/${BINARY_NAME}" ] || fail "release archive did not contain ${BINARY_NAME}"

install -m 755 "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"

log "Installed Orbit to ${INSTALL_DIR}/${BINARY_NAME}"
"${INSTALL_DIR}/${BINARY_NAME}" --version

case ":$PATH:" in
  *:"$INSTALL_DIR":*)
    ;;
  *)
    log
    log "Add Orbit to your PATH by adding this line to your shell profile:"
    log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac

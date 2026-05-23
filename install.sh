#!/bin/sh

set -eu

REPO="${ORBIT_INSTALL_REPO:-danieljhkim/orbit}"
BINARY_NAME="orbit"
CHECKSUM_FILE="orbit-checksums.txt"
CHECKSUM_SIGNATURE_FILE="orbit-checksums.txt.sig"
INSTALL_DIR="${ORBIT_INSTALL_DIR:-$HOME/.orbit/bin}"

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'orbit installer: %s\n' "$*" >&2
  exit 1
}

warn() {
  printf 'orbit installer: %s\n' "$*" >&2
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

write_trusted_public_key() {
  destination="$1"

  if [ -n "${ORBIT_RELEASE_PUBLIC_KEY_FILE:-}" ]; then
    [ "${ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE:-}" = "1" ] \
      || fail "ORBIT_RELEASE_PUBLIC_KEY_FILE requires ORBIT_RELEASE_PUBLIC_KEY_FILE_ACKNOWLEDGE_TRUST_CHANGE=1"
    [ -f "$ORBIT_RELEASE_PUBLIC_KEY_FILE" ] || fail "ORBIT_RELEASE_PUBLIC_KEY_FILE does not exist: $ORBIT_RELEASE_PUBLIC_KEY_FILE"
    warn "ORBIT_RELEASE_PUBLIC_KEY_FILE=$ORBIT_RELEASE_PUBLIC_KEY_FILE set; trusting replacement release signing key"
    cat "$ORBIT_RELEASE_PUBLIC_KEY_FILE" > "$destination"
    return
  fi

  cat > "$destination" <<'EOF'
-----BEGIN PUBLIC KEY-----
MIIBojANBgkqhkiG9w0BAQEFAAOCAY8AMIIBigKCAYEAuZ8vNa+DusYhrFBXNhBh
RSqn81AYe7tYEtCKImWGuy/6ziMHqDzDKHSku0sBMwcdLXBzI0RjNBacLCbbYr4H
icmYrsKqqfLGf+CWfrqDqY9d3hwUPtVMRp/ynVNW6nwKAmNl5dTgUc6ZBAZTtQtt
qwMD1JIOsrJ3vVDL9o3alcXcg/RyL0pGUo+vep2QZOjXnCGoJN3NeytQHag3zJyd
Wq4psc7j2H1Nb5EoyY/I/7vpdwME3Mrv2ffwtDmr0/+73q1yWUDf4btY9Ba7sOhE
Ir2UHm3bEboo1ErAYjjiDDuF/NjzZcZpJtuNbdj0vI7pHDyDZ7sKiEX7RkUO+e2c
IouiSfRJRrwnjpuergrq3ehNjkxcn5dFST1l23FOXGsy4F7ilrF6P9cgaAsE8dc7
CS9YgUE1ErfGJLZtfDDGKs6+E+7JiC1C3z7xwmfzOgv9gEvSlfrx2BbGl8esypKm
pYZDkW2dLqPeFj/WwGhZoYFHv0GOMIWdi6FNriQdkn4RAgMBAAE=
-----END PUBLIC KEY-----
EOF
}

verify_checksum_signature() {
  checksum_path="$1"
  signature_path="$2"
  public_key_path="$3"

  openssl dgst -sha256 -verify "$public_key_path" -signature "$signature_path" "$checksum_path" >/dev/null 2>&1 \
    || fail "release checksum signature verification failed for ${CHECKSUM_FILE}"
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

validate_archive_members() {
  archive_path="$1"
  member_list="${TMP_DIR}/archive-members.txt"
  member=""
  member_count=0

  tar -tzf "$archive_path" > "$member_list" || fail "could not inspect release archive"
  while IFS= read -r archive_member; do
    member_count=$((member_count + 1))
    if [ "$member_count" -eq 1 ]; then
      member="$archive_member"
    fi
  done < "$member_list"
  [ "$member_count" = "1" ] || fail "release archive must contain only ${BINARY_NAME}"

  case "$member" in
    "$BINARY_NAME")
      ;;
    "" | /* | *"/.."* | "../"* | *"/../"* | *".."* )
      fail "unsafe release archive member: ${member:-<empty>}"
      ;;
    *)
      fail "unexpected release archive member: $member"
      ;;
  esac
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
need_cmd openssl
need_cmd tar

TARGET="$(resolve_target)"
ARCHIVE_NAME="orbit-${TARGET}.tar.gz"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/orbit-install.XXXXXX")"

cleanup() {
  rm -rf "$TMP_DIR"
}

trap cleanup EXIT HUP INT TERM

if [ -n "${ORBIT_INSTALL_BASE_URL:-}" ]; then
  BASE_URL="${ORBIT_INSTALL_BASE_URL%/}"
  VERSION_LABEL="${ORBIT_VERSION:-custom}"
elif [ -n "${ORBIT_VERSION:-}" ]; then
  RELEASE_TAG="$(normalize_tag "$ORBIT_VERSION")"
  BASE_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}"
  VERSION_LABEL="$RELEASE_TAG"
else
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
  VERSION_LABEL="latest"
fi

ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"
CHECKSUM_PATH="${TMP_DIR}/${CHECKSUM_FILE}"
SIGNATURE_PATH="${TMP_DIR}/${CHECKSUM_SIGNATURE_FILE}"
PUBLIC_KEY_PATH="${TMP_DIR}/orbit-release-signing.pub"

log "Downloading Orbit ${VERSION_LABEL} for ${TARGET}..."
download "${BASE_URL}/${CHECKSUM_FILE}" "$CHECKSUM_PATH"
download "${BASE_URL}/${CHECKSUM_SIGNATURE_FILE}" "$SIGNATURE_PATH"
write_trusted_public_key "$PUBLIC_KEY_PATH"
verify_checksum_signature "$CHECKSUM_PATH" "$SIGNATURE_PATH" "$PUBLIC_KEY_PATH"
download "${BASE_URL}/${ARCHIVE_NAME}" "$ARCHIVE_PATH"

EXPECTED_SHA="$(awk -v asset="$ARCHIVE_NAME" '$2 == asset { print $1 }' "$CHECKSUM_PATH")"
[ -n "$EXPECTED_SHA" ] || fail "checksum entry for ${ARCHIVE_NAME} was not found in ${CHECKSUM_FILE}"

ACTUAL_SHA="$(compute_sha256 "$ARCHIVE_PATH")"

if [ "$EXPECTED_SHA" != "$ACTUAL_SHA" ]; then
  fail "checksum verification failed for ${ARCHIVE_NAME} (expected ${EXPECTED_SHA}, got ${ACTUAL_SHA})"
fi

mkdir -p "$INSTALL_DIR"
validate_archive_members "$ARCHIVE_PATH"
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR" "$BINARY_NAME"
[ ! -L "${TMP_DIR}/${BINARY_NAME}" ] || fail "release archive member is a symlink: ${BINARY_NAME}"
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

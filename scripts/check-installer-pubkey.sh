#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
canonical_key="$repo_root/plugin/npm/release-signing.pub"
extracted_key="$(mktemp)"
trap 'rm -f "$extracted_key"' EXIT

key_count="$(awk '/^-----BEGIN PUBLIC KEY-----$/ { count++ } END { print count + 0 }' "$repo_root/install.sh")"
if [ "$key_count" != "1" ]; then
  echo "check-installer-pubkey: expected exactly one public key block in install.sh, found $key_count" >&2
  exit 1
fi

awk '
  /^-----BEGIN PUBLIC KEY-----$/ { in_key = 1 }
  in_key { print }
  /^-----END PUBLIC KEY-----$/ { found = 1; in_key = 0 }
  END { if (!found) exit 1 }
' "$repo_root/install.sh" > "$extracted_key" || {
  echo "check-installer-pubkey: could not extract release signing public key from install.sh" >&2
  exit 1
}

if ! diff -u "$canonical_key" "$extracted_key"; then
  echo "check-installer-pubkey: install.sh public key must match plugin/npm/release-signing.pub" >&2
  exit 1
fi

echo "check-installer-pubkey: ok"

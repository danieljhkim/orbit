#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
canonical_key="$repo_root/plugin/npm/release-signing.pub"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

extract_pem_blocks() {
  local source="$1"
  local destination="$2"

  awk '
    /-----BEGIN PUBLIC KEY-----/ { in_key = 1 }
    in_key { print }
    /-----END PUBLIC KEY-----/ { found = 1; in_key = 0 }
    END { if (!found) exit 1 }
  ' "$source" > "$destination"
}

assert_contains_canonical_key() {
  local label="$1"
  local extracted_keys="$2"

  awk '
    NR == FNR {
      canonical = canonical $0 "\n"
      next
    }
    {
      extracted = extracted $0 "\n"
    }
    END {
      exit(index(extracted, canonical) ? 0 : 1)
    }
  ' "$canonical_key" "$extracted_keys" || {
    echo "check-installer-pubkey: $label must include plugin/npm/release-signing.pub" >&2
    exit 1
  }
}

install_keys="$tmp_dir/install-sh-keys.pem"
npm_keys="$tmp_dir/npm-install-keys.pem"
extract_pem_blocks "$repo_root/install.sh" "$install_keys" || {
  echo "check-installer-pubkey: could not extract release signing public keys from install.sh" >&2
  exit 1
}
extract_pem_blocks "$repo_root/plugin/npm/scripts/install-binary.js" "$npm_keys" || {
  echo "check-installer-pubkey: could not extract release signing public keys from install-binary.js" >&2
  exit 1
}

install_key_count="$(awk '/-----BEGIN PUBLIC KEY-----/ { count++ } END { print count + 0 }' "$install_keys")"
npm_key_count="$(awk '/-----BEGIN PUBLIC KEY-----/ { count++ } END { print count + 0 }' "$npm_keys")"
if [ "$install_key_count" -lt 2 ]; then
  echo "check-installer-pubkey: expected at least two public key blocks in install.sh, found $install_key_count" >&2
  exit 1
fi
if [ "$npm_key_count" -lt 2 ]; then
  echo "check-installer-pubkey: expected at least two public key blocks in install-binary.js, found $npm_key_count" >&2
  exit 1
fi

assert_contains_canonical_key "install.sh" "$install_keys"
assert_contains_canonical_key "install-binary.js" "$npm_keys"

grep -q "orbit-release-2026-05-primary" "$repo_root/install.sh"
grep -q "orbit-release-2026-05-successor" "$repo_root/install.sh"
grep -q "orbit-release-2026-05-primary" "$repo_root/plugin/npm/scripts/install-binary.js"
grep -q "orbit-release-2026-05-successor" "$repo_root/plugin/npm/scripts/install-binary.js"
grep -q "notAfter: '2027-12-31'" "$repo_root/plugin/npm/scripts/install-binary.js"
grep -q "notAfter: '2028-12-31'" "$repo_root/plugin/npm/scripts/install-binary.js"
grep -q "2027-12-31" "$repo_root/install.sh"
grep -q "2028-12-31" "$repo_root/install.sh"

echo "check-installer-pubkey: ok"

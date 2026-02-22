#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

fail=0

has_dep() {
  local manifest="$1"
  local dep="$2"
  rg -n "^[[:space:]]*${dep}[[:space:]]*=" "$manifest" >/dev/null
}

require_dep() {
  local manifest="$1"
  local dep="$2"
  if ! has_dep "$manifest" "$dep"; then
    echo "missing required dependency '${dep}' in ${manifest}"
    fail=1
  fi
}

forbid_dep() {
  local manifest="$1"
  local dep="$2"
  if has_dep "$manifest" "$dep"; then
    echo "forbidden dependency '${dep}' found in ${manifest}"
    fail=1
  fi
}

core_manifest="$repo_root/orbit-core/Cargo.toml"
cli_manifest="$repo_root/orbit-cli/Cargo.toml"
types_manifest="$repo_root/orbit-types/Cargo.toml"

# Required downward edges from core.
for dep in orbit-policy orbit-exec orbit-tools orbit-store orbit-types; do
  require_dep "$core_manifest" "$dep"
done

# CLI must only depend on core from workspace crates.
for dep in orbit-store orbit-tools orbit-exec orbit-policy orbit-types; do
  forbid_dep "$cli_manifest" "$dep"
done

# types must not depend on any workspace crate.
if rg -n "^[[:space:]]*orbit-[a-z-]+[[:space:]]*=" "$types_manifest" >/dev/null; then
  echo "orbit-types cannot depend on workspace crates"
  rg -n "^[[:space:]]*orbit-[a-z-]+[[:space:]]*=" "$types_manifest"
  fail=1
fi

# Lower layers must not depend upward on core/cli.
for crate in orbit-store orbit-policy orbit-exec orbit-tools; do
  manifest="$repo_root/${crate}/Cargo.toml"
  forbid_dep "$manifest" "orbit-core"
  forbid_dep "$manifest" "orbit-cli"
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "dependency direction guard passed"

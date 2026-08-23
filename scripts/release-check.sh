#!/usr/bin/env bash
# Verify release-version lockstep across Cargo, the npm proxy, npm, and GitHub.
#
# Exits 0 when every reachable source agrees, 1 on drift, and 2 on a missing
# local prerequisite. Remote registry/release checks remain soft so the target
# is useful from a fresh checkout without credentials.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

NPM_PKG="@orbit-tools/cli"
CARGO_TOML="Cargo.toml"
NPM_PACKAGE_JSON="npm/package.json"

require_bin() {
  local bin="$1"
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "release-check: required binary '$bin' not on PATH" >&2
    exit 2
  fi
}

require_bin jq

for file in "$CARGO_TOML" "$NPM_PACKAGE_JSON"; do
  if [[ ! -f "$file" ]]; then
    echo "release-check: $file not found (run from repo root)" >&2
    exit 2
  fi
done

cargo_package_version="$(
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && /^version[[:space:]]*=/ {
      value = $0
      sub(/^[^=]*=[[:space:]]*/, "", value)
      gsub(/"/, "", value)
      print value
      exit
    }
  ' "$CARGO_TOML"
)"
npm_package_version="$(jq -r .version "$NPM_PACKAGE_JSON")"

if [[ -z "$cargo_package_version" ]]; then
  echo "release-check: $CARGO_TOML has no [workspace.package] version" >&2
  exit 2
fi
if [[ -z "$npm_package_version" || "$npm_package_version" == "null" ]]; then
  echo "release-check: $NPM_PACKAGE_JSON has no .version field" >&2
  exit 2
fi

npm_registry_version=""
if command -v npm >/dev/null 2>&1; then
  if version="$(npm view "$NPM_PKG" version 2>/dev/null)"; then
    npm_registry_version="$version"
  else
    echo "release-check: npm view $NPM_PKG version failed (registry unreachable?)" >&2
  fi
else
  echo "release-check: npm not on PATH; skipping registry check" >&2
fi

gh_tag_version=""
if command -v gh >/dev/null 2>&1; then
  if tag="$(gh release list -L 1 --json tagName -q '.[0].tagName' 2>/dev/null)"; then
    gh_tag_version="${tag#v}"
  else
    echo "release-check: gh release list failed (not authenticated or no releases?)" >&2
  fi
else
  echo "release-check: gh not on PATH; skipping GitHub Release check" >&2
fi

printf '%-32s %s\n' "$CARGO_TOML [workspace.package]" "$cargo_package_version"
printf '%-32s %s\n' "$NPM_PACKAGE_JSON" "$npm_package_version"
printf '%-32s %s\n' "npm view $NPM_PKG" "${npm_registry_version:-<skipped>}"
printf '%-32s %s\n' "gh release list -L 1" "${gh_tag_version:-<skipped>}"

drift=0
compare_version() {
  local source="$1"
  local version="$2"
  if [[ -n "$version" && "$cargo_package_version" != "$version" ]]; then
    echo "DRIFT: $CARGO_TOML ($cargo_package_version) != $source ($version)" >&2
    drift=1
  fi
}

compare_version "$NPM_PACKAGE_JSON" "$npm_package_version"
compare_version "npm view $NPM_PKG" "$npm_registry_version"
compare_version "latest gh release tag" "$gh_tag_version"

if [[ "$drift" -ne 0 ]]; then
  cat >&2 <<EOF

release-check failed. See docs/runbooks/release.md for the procedure.
Cargo, npm package, npm registry, and GitHub Release versions must agree.
EOF
  exit 1
fi

if [[ -z "$npm_registry_version" || -z "$gh_tag_version" ]]; then
  echo "release-check: local sources agree on $cargo_package_version; remote checks were skipped." >&2
  exit 0
fi

echo "release-check: all sources agree on $cargo_package_version"

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

fail=0

allowed_internal_deps() {
  case "$1" in
    orbit-common)
      echo ""
      ;;
    orbit-registry)
      echo "orbit-common"
      ;;
    orbit-policy | orbit-exec | orbit-store | orbit-search)
      echo "orbit-common"
      ;;
    orbit-search-companion)
      echo "orbit-common orbit-search"
      ;;
    orbit-graph-extract)
      echo ""
      ;;
    orbit-graph)
      echo "orbit-graph-extract"
      ;;
    orbit-graph-cli)
      echo "orbit-graph orbit-graph-extract"
      ;;
    orbit-tools)
      echo "orbit-common orbit-exec orbit-policy"
      ;;
    orbit-agent)
      echo "orbit-common orbit-tools"
      ;;
    orbit-engine)
      echo "orbit-agent orbit-common orbit-exec orbit-store orbit-tools"
      ;;
    orbit-core)
      echo "orbit-common orbit-search orbit-engine orbit-policy orbit-store orbit-tools"
      ;;
    orbit-mcp)
      echo "orbit-common orbit-graph orbit-graph-extract orbit-tools"
      ;;
    orbit-dashboard)
      echo "orbit-common orbit-core"
      ;;
    orbit-cli)
      echo "orbit-common orbit-core orbit-mcp orbit-dashboard"
      ;;
    *)
      return 1
      ;;
  esac
}

contains_word() {
  local haystack="$1"
  local needle="$2"
  for word in $haystack; do
    if [[ "$word" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

load_workspace_crates() {
  cargo metadata --format-version 1 --no-deps --manifest-path "$repo_root/Cargo.toml" |
    python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
workspace_members = set(metadata["workspace_members"])
workspace_crates = sorted(
    (package["name"], package["manifest_path"])
    for package in metadata["packages"]
    if package["id"] in workspace_members
    and package["name"].startswith("orbit-")
)
for crate, manifest_path in workspace_crates:
    print(f"{crate}\t{manifest_path}")
'
}

workspace_crates=()
workspace_manifests=()
while IFS=$'\t' read -r crate manifest; do
  if [[ -n "$crate" ]]; then
    workspace_crates+=("$crate")
    workspace_manifests+=("$manifest")
  fi
done < <(load_workspace_crates)

if [[ "${#workspace_crates[@]}" -eq 0 ]]; then
  echo "no orbit workspace crates discovered from cargo metadata"
  exit 1
fi

for index in "${!workspace_crates[@]}"; do
  crate="${workspace_crates[$index]}"
  manifest="${workspace_manifests[$index]}"
  if [[ ! -f "$manifest" ]]; then
    echo "missing manifest for ${crate}: ${manifest}"
    fail=1
    continue
  fi

  if ! allowed="$(allowed_internal_deps "$crate")"; then
    echo "missing dependency direction policy for workspace crate '${crate}'"
    fail=1
    continue
  fi

  while IFS= read -r dep; do
    if [[ -n "$dep" ]] && ! contains_word "$allowed" "$dep"; then
      echo "forbidden dependency '${dep}' found in ${manifest}"
      echo "  allowed internal deps for ${crate}: ${allowed:-<none>}"
      fail=1
    fi
  done < <(
    rg -o "^[[:space:]]*orbit-[a-z-]+[[:space:]]*=" "$manifest" |
      sed -E 's/^[[:space:]]*(orbit-[a-z-]+)[[:space:]]*=.*/\1/'
  )
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "dependency direction guard passed"

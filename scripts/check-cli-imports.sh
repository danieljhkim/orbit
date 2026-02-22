#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cli_src_dir="$repo_root/orbit-cli/src"

forbidden=(
  "orbit_store"
  "orbit_tools"
  "orbit_exec"
  "orbit_policy"
  "orbit_types"
)

failed=0
for crate in "${forbidden[@]}"; do
  if rg -n "\\b${crate}::" "$cli_src_dir" >/dev/null; then
    echo "forbidden import reference found in orbit-cli: ${crate}::"
    rg -n "\\b${crate}::" "$cli_src_dir"
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi

echo "orbit-cli import guard passed"

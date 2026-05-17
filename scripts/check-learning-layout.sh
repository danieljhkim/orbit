#!/usr/bin/env bash
set -euo pipefail

repo_root="${1:-${ORBIT_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}}"
learnings_dir="$repo_root/.orbit/learnings"

if [[ ! -d "$learnings_dir" ]]; then
  exit 0
fi

flat_file="$(find "$learnings_dir" -maxdepth 1 -type f -name 'L*.yaml' -print -quit)"
if [[ -n "$flat_file" ]]; then
  echo "error: flat legacy learning file at $flat_file; run orbit learning migrate-layout" >&2
  exit 1
fi

#!/usr/bin/env bash
set -euo pipefail

mode="${1:?usage: run.sh capture|compare <fixture-workspace> <baseline-dir>}"
fixture="${2:?usage: run.sh capture|compare <fixture-workspace> <baseline-dir>}"
baseline="${3:?usage: run.sh capture|compare <fixture-workspace> <baseline-dir>}"

if [[ "$mode" != "capture" && "$mode" != "compare" ]]; then
  echo "mode must be capture or compare" >&2
  exit 2
fi

mkdir -p "$baseline"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
work="$tmpdir/workspace"
mkdir -p "$work"
rsync -a \
  --exclude .git \
  --exclude target \
  --exclude node_modules \
  "$fixture"/ "$work"/

run_tool() {
  local name="$1"
  local input="$2"
  orbit tool run "$name" --input "$input"
}

cd "$work"

run_tool orbit.search '{"query":"main","kind":"all","limit":10,"model":"codex"}' > "$tmpdir/search_all.json"
run_tool orbit.search '{"query":"main","kind":"task","limit":10,"model":"codex"}' > "$tmpdir/search_tasks.json"
run_tool orbit.search '{"query":"main","kind":"doc","limit":10,"model":"codex"}' > "$tmpdir/search_docs.json"
run_tool orbit.task.list '{"limit":10,"model":"codex"}' > "$tmpdir/tasks.json"
run_tool orbit.docs.list '{"model":"codex"}' > "$tmpdir/docs.json"
run_tool orbit.learning.list '{"model":"codex"}' > "$tmpdir/learnings.json"
run_tool orbit.adr.list '{"model":"codex"}' > "$tmpdir/adrs.json"

if [[ "$mode" == "capture" ]]; then
  cp "$tmpdir"/*.json "$baseline"/
  exit 0
fi

for current in "$tmpdir"/*.json; do
  name="$(basename "$current")"
  cmp --silent "$baseline/$name" "$current" || {
    echo "equivalence mismatch: $name" >&2
    diff -u "$baseline/$name" "$current" >&2 || true
    exit 1
  }
done

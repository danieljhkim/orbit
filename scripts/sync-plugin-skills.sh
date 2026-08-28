#!/usr/bin/env bash
# Materialize the agent plugin skill tree from Orbit's canonical embedded assets.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
source_dir="$repo_root/crates/orbit-core/assets/skills/orbit"
target_dir="$repo_root/plugin/skills/orbit"

if [[ ! -f "$source_dir/SKILL.md" ]]; then
  echo "sync-plugin-skills: canonical skill is missing: $source_dir/SKILL.md" >&2
  exit 1
fi

if [[ "${1:-}" == "--check" ]]; then
  if [[ "$#" -ne 1 ]]; then
    echo "usage: $0 [--check]" >&2
    exit 2
  fi
  if ! diff -qr "$source_dir" "$target_dir" >/dev/null; then
    echo "sync-plugin-skills: plugin/skills/orbit drifted from crates/orbit-core/assets/skills/orbit" >&2
    diff -ru "$source_dir" "$target_dir" || true
    exit 1
  fi
  echo "sync-plugin-skills: committed plugin skill mirror matches canonical assets"
  exit 0
fi
if [[ "$#" -ne 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

mkdir -p "$repo_root/plugin/skills"
rm -rf "$target_dir"
cp -R "$source_dir" "$target_dir"

echo "sync-plugin-skills: materialized plugin/skills/orbit from crates/orbit-core/assets/skills/orbit"

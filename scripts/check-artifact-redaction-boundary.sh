#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

targets=(
  "crates/orbit-core/src/runtime/orbit_tool_host"
  "crates/orbit-tools/src/builtin/orbit/adr"
  "crates/orbit-tools/src/builtin/orbit/learning"
  "crates/orbit-store/src/file/adr_store"
  "crates/orbit-store/src/file/learning_store"
)

if hits="$(rg -n 'fn[[:space:]]+redact_[A-Za-z0-9_]*' "${targets[@]}" --glob '*.rs')"; then
  cat >&2 <<EOF
Artifact write surfaces must use orbit_common::utility::redaction instead of
defining local redact_* helpers:

$hits
EOF
  exit 1
fi

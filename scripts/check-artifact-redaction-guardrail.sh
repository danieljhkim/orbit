#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

targets=(
  "crates/orbit-core/src/runtime/orbit_tool_host/adr_tools.rs"
  "crates/orbit-core/src/runtime/orbit_tool_host/learning_tools.rs"
  "crates/orbit-core/src/runtime/orbit_tool_host/task_tools.rs"
  "crates/orbit-core/src/runtime/orbit_tool_host/review_threads.rs"
  "crates/orbit-core/src/runtime/orbit_tool_host/friction_tools.rs"
  "crates/orbit-tools/src/builtin/orbit/adr"
  "crates/orbit-tools/src/builtin/orbit/learning"
  "crates/orbit-tools/src/builtin/orbit/task"
  "crates/orbit-tools/src/builtin/orbit/review_thread"
  "crates/orbit-tools/src/builtin/orbit/friction"
)

if rg -n 'fn\s+redact_' "${targets[@]}"; then
  cat >&2 <<'MSG'
Artifact write redaction must flow through orbit_common::utility::redaction and the shared tool-host policy.
Do not add surface-local `fn redact_*` helpers for ADR, learning, task, review-thread, or friction tools.
MSG
  exit 1
fi

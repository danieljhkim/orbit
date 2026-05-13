#!/usr/bin/env bash
# Orbit SessionStart hook: emit a workspace-init prompt when the user's cwd is
# not inside an initialized Orbit workspace. Pure filesystem walk — no `orbit`
# binary dependency and no state mutation. Mirrors the discovery rules in
# `crates/orbit-core/src/runtime/resolve.rs` (find_orbit_dir_walk_up +
# is_initialized_orbit_root).
set -eu

target_dir=""
if [ -n "${CLAUDE_PROJECT_DIR:-}" ]; then
  target_dir="$CLAUDE_PROJECT_DIR"
elif [ ! -t 0 ]; then
  payload=$(cat || true)
  if [ -n "$payload" ] && command -v python3 >/dev/null 2>&1; then
    parsed=$(printf '%s' "$payload" | python3 -c \
      'import json,sys
try:
    print(json.loads(sys.stdin.read()).get("cwd",""))
except Exception:
    pass' 2>/dev/null || true)
    if [ -n "$parsed" ]; then
      target_dir="$parsed"
    fi
  fi
fi
if [ -z "$target_dir" ]; then
  target_dir="$PWD"
fi

global_orbit="${HOME:-/}/.orbit"

is_initialized_orbit_dir() {
  candidate="$1"
  [ -d "$candidate" ] || return 1
  # Skip the user's global ~/.orbit — it is not a workspace.
  if [ "$candidate" = "$global_orbit" ]; then
    return 1
  fi
  if [ -f "$candidate/config.toml" ]; then
    return 0
  fi
  if [ -d "$candidate/resources" ] && [ -d "$candidate/tasks" ] && [ -d "$candidate/state" ]; then
    return 0
  fi
  return 1
}

current="$target_dir"
while :; do
  if is_initialized_orbit_dir "$current/.orbit"; then
    exit 0
  fi
  parent=$(dirname -- "$current")
  if [ "$parent" = "$current" ]; then
    break
  fi
  current="$parent"
done

# Uninitialized: surface a single systemMessage to the agent.
cat <<'JSON'
{
  "continue": true,
  "suppressOutput": false,
  "systemMessage": "Orbit is not initialized in this workspace. The Orbit MCP server is connected but exposes zero tools until a workspace exists. Tell the user to run `orbit init` once (creates ~/.orbit), then `orbit workspace init` from this repo root — then restart Claude Code so the plugin hooks reload."
}
JSON

exit 0

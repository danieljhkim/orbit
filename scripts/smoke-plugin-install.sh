#!/usr/bin/env bash
# End-to-end smoke for the agent plugin install orbit chain.
#
# Runs against the *published* @orbit-tools/cli@latest from npm (not the
# local working tree) so it catches version drift between the npm proxy
# and the GitHub Release binary it fetches.
#
# Steps:
#   1. npx -y @orbit-tools/cli@latest --version
#        -> exercises postinstall: tarball download + sha256 verification
#   2. drive `orbit mcp serve` over stdio with a JSON-RPC handshake
#      (initialize + tools/list) and assert at least one `orbit.*` tool
#      appears in the response.
#   3. install the repository Codex plugin into an isolated CODEX_HOME,
#      render a fresh Codex task prompt to verify Orbit skill discovery, and
#      call the read-only orbit.task.list MCP tool through the installed
#      plugin transport.
#
# Pass: exit 0. Fail: non-zero with the relevant stderr captured.
# Supported: macOS arm64 / x86_64, Linux x86_64 / arm64. Not Windows.
set -euo pipefail

require_bin() {
  local bin="$1"
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "smoke-plugin-install: required binary '$bin' not on PATH" >&2
    exit 2
  fi
}

require_bin node
require_bin npx
require_bin npm
require_bin python3
require_bin codex

case "$(uname -s)" in
  Darwin|Linux) ;;
  *)
    echo "smoke-plugin-install: unsupported OS '$(uname -s)' — supported: Darwin, Linux" >&2
    exit 2
    ;;
esac

NPM_PKG="@orbit-tools/cli"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
TMPDIR_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

# Cache npx installs inside the temp dir so we exercise a clean download.
export npm_config_cache="$TMPDIR_ROOT/npm-cache"
mkdir -p "$npm_config_cache"

# Sandbox HOME so `orbit init` writes to the temp tree instead of the
# runner/user's real ~/.orbit. Without this, repeat runs locally would
# accumulate state in the developer's global Orbit root.
SMOKE_HOME="$TMPDIR_ROOT/home"
mkdir -p "$SMOKE_HOME"
export HOME="$SMOKE_HOME"

# Pick a timeout binary. macOS runners ship neither `timeout` nor `gtimeout`
# unless coreutils is installed; fall back to perl, which is preinstalled on
# both macOS-15 and ubuntu-22.04 GitHub runners. Perl's `alarm` survives exec,
# so the target process gets SIGALRM at the deadline and exits with rc 142.
if command -v timeout >/dev/null 2>&1; then
  TIMEOUT_KIND=gnu
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT_KIND=gnu_g
elif command -v perl >/dev/null 2>&1; then
  TIMEOUT_KIND=perl
else
  echo "smoke-plugin-install: need timeout, gtimeout, or perl on PATH" >&2
  exit 2
fi

run_with_timeout() {
  local secs="$1"; shift
  case "$TIMEOUT_KIND" in
    gnu) timeout "$secs" "$@" ;;
    gnu_g) gtimeout "$secs" "$@" ;;
    perl) perl -e '$t=shift @ARGV; alarm $t; exec @ARGV or die "exec failed: $!\n"' "$secs" "$@" ;;
  esac
}

assert_jsonrpc_tool_call_ok() {
  local jsonrpc_output="$1"
  local call_id="$2"
  python3 - "$jsonrpc_output" "$call_id" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
call_id = int(sys.argv[2])
target = None
for line in path.read_text(encoding="utf-8").splitlines():
    line = line.strip()
    if not line:
        continue
    try:
        payload = json.loads(line)
    except json.JSONDecodeError:
        continue
    if payload.get("id") == call_id:
        target = payload
        break

if target is None:
    print(f"FAIL: no JSON-RPC response with id={call_id}", file=sys.stderr)
    raise SystemExit(1)
if "error" in target:
    print(f"FAIL: JSON-RPC response id={call_id} returned error: {target['error']}", file=sys.stderr)
    raise SystemExit(1)
print(f"JSON-RPC response id={call_id} succeeded")
PY
}

echo "--- step 1: npx -y $NPM_PKG@latest --version ---"
VERSION_OUT="$TMPDIR_ROOT/version.txt"
if ! npx -y "$NPM_PKG@latest" --version >"$VERSION_OUT" 2>"$TMPDIR_ROOT/version.err"; then
  echo "FAIL: npx -y $NPM_PKG@latest --version exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/version.err" >&2
  exit 1
fi
binary_version="$(tr -d '[:space:]' <"$VERSION_OUT" || true)"
if [[ -z "$binary_version" ]]; then
  echo "FAIL: orbit --version produced no output" >&2
  cat "$TMPDIR_ROOT/version.err" >&2
  exit 1
fi
echo "orbit --version => $binary_version"

# Also confirm the npm-registry version, for the smoke report.
npm_version="$(npm view "$NPM_PKG" version 2>/dev/null || echo '<unknown>')"
echo "$NPM_PKG@latest on npm => $npm_version"

echo "--- step 2: orbit init + workspace init ---"
# `orbit mcp serve` deliberately refuses to bootstrap a workspace (see
# OrbitRuntime::try_initialize_existing) — so without these two commands the
# MCP server attaches but serves an empty tool surface. Initializing first
# matches the documented /plugin install orbit flow.
if ! npx -y "$NPM_PKG@latest" init --non-interactive >"$TMPDIR_ROOT/init.out" 2>"$TMPDIR_ROOT/init.err"; then
  echo "FAIL: orbit init exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/init.err" >&2
  exit 1
fi
echo "orbit init => OK ($SMOKE_HOME/.orbit)"

WORKSPACE_DIR="$TMPDIR_ROOT/workspace"
mkdir -p "$WORKSPACE_DIR"
if ! ( cd "$WORKSPACE_DIR" && npx -y "$NPM_PKG@latest" workspace init ) \
     >"$TMPDIR_ROOT/ws-init.out" 2>"$TMPDIR_ROOT/ws-init.err"; then
  echo "FAIL: orbit workspace init exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/ws-init.err" >&2
  exit 1
fi
echo "orbit workspace init => OK ($WORKSPACE_DIR/.orbit)"

echo "--- step 3: MCP handshake over stdio ---"
RPC_IN="$TMPDIR_ROOT/rpc-in.txt"
RPC_OUT="$TMPDIR_ROOT/rpc-out.txt"
RPC_ERR="$TMPDIR_ROOT/rpc-err.txt"

cat >"$RPC_IN" <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-plugin-install","version":"0.0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
EOF

# Feed the requests, then keep stdin open briefly so the server has time
# to flush its responses before EOF closes the channel. `mcp serve` runs
# from inside the initialized workspace so it can discover `.orbit/`.
{
  cat "$RPC_IN"
  sleep 5
} | ( cd "$WORKSPACE_DIR" && run_with_timeout 120 npx -y "$NPM_PKG@latest" mcp serve ) \
     >"$RPC_OUT" 2>"$RPC_ERR" || rc=$?
rc="${rc:-0}"

# Accepted post-handshake exit codes:
#   0   — server saw stdin EOF and exited cleanly (the expected happy path).
#   124 — GNU `timeout` killed it (we never send `shutdown`).
#   143 — SIGTERM (128+15), via GNU `timeout --signal` or external kill.
#   142 — SIGALRM (128+14), via the perl-based timeout fallback on macOS.
if [[ "$rc" -ne 0 && "$rc" -ne 124 && "$rc" -ne 142 && "$rc" -ne 143 ]]; then
  echo "FAIL: mcp serve exited with $rc" >&2
  echo "--- stderr ---" >&2
  cat "$RPC_ERR" >&2
  echo "--- stdout ---" >&2
  cat "$RPC_OUT" >&2
  exit 1
fi

if ! grep -q '"jsonrpc":"2.0"' "$RPC_OUT"; then
  echo "FAIL: no JSON-RPC frames in mcp serve stdout" >&2
  echo "--- stdout ---" >&2
  cat "$RPC_OUT" >&2
  echo "--- stderr ---" >&2
  cat "$RPC_ERR" >&2
  exit 1
fi

# tools/list response should advertise at least one orbit_* tool.
# Tool names are emitted with underscores on the MCP wire (orbit-mcp's
# sanitize_tool_name replaces `.` with `_` for client compatibility), so the
# canonical `orbit.task.show` selector arrives here as `orbit_task_show`.
if ! grep -q '"orbit_' "$RPC_OUT"; then
  echo "FAIL: tools/list response contained no orbit_* tools" >&2
  echo "--- stdout ---" >&2
  cat "$RPC_OUT" >&2
  echo "--- stderr ---" >&2
  cat "$RPC_ERR" >&2
  exit 1
fi

orbit_tool_count="$(grep -o '"orbit_[a-z_]*"' "$RPC_OUT" | sort -u | wc -l | tr -d '[:space:]')"
echo "tools/list returned $orbit_tool_count distinct orbit_* tools"

echo "--- step 4: Codex plugin install in isolated CODEX_HOME ---"
CODEX_HOME_DIR="$TMPDIR_ROOT/codex-home"
mkdir -p "$CODEX_HOME_DIR"
CODEX_ENV=(env CODEX_HOME="$CODEX_HOME_DIR")

if ! "${CODEX_ENV[@]}" codex plugin marketplace add "$repo_root" --json \
     >"$TMPDIR_ROOT/codex-marketplace-add.json" 2>"$TMPDIR_ROOT/codex-marketplace-add.err"; then
  echo "FAIL: codex plugin marketplace add $repo_root exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/codex-marketplace-add.err" >&2
  exit 1
fi
python3 - "$TMPDIR_ROOT/codex-marketplace-add.json" <<'PY'
import json
import sys
payload = json.load(open(sys.argv[1], encoding="utf-8"))
if payload.get("marketplaceName") != "orbit":
    raise SystemExit(f"FAIL: expected marketplaceName=orbit, got {payload!r}")
print("codex marketplace => orbit")
PY

if ! "${CODEX_ENV[@]}" codex plugin add orbit@orbit --json \
     >"$TMPDIR_ROOT/codex-plugin-add.json" 2>"$TMPDIR_ROOT/codex-plugin-add.err"; then
  echo "FAIL: codex plugin add orbit@orbit exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/codex-plugin-add.err" >&2
  exit 1
fi
python3 - "$TMPDIR_ROOT/codex-plugin-add.json" <<'PY'
import json
import sys
payload = json.load(open(sys.argv[1], encoding="utf-8"))
if payload.get("name") != "orbit" or payload.get("marketplaceName") != "orbit":
    raise SystemExit(f"FAIL: expected installed orbit@orbit, got {payload!r}")
print(f"codex plugin installed => {payload.get('installedPath')}")
PY

if ! "${CODEX_ENV[@]}" codex mcp list --json \
     >"$TMPDIR_ROOT/codex-mcp-list.json" 2>"$TMPDIR_ROOT/codex-mcp-list.err"; then
  echo "FAIL: codex mcp list exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/codex-mcp-list.err" >&2
  exit 1
fi
CODEX_MCP_TRANSPORT="$TMPDIR_ROOT/codex-mcp-transport.json"
python3 - "$TMPDIR_ROOT/codex-mcp-list.json" "$CODEX_MCP_TRANSPORT" <<'PY'
import json
import sys
from pathlib import Path

servers = json.load(open(sys.argv[1], encoding="utf-8"))
transport_path = Path(sys.argv[2])
orbit = next((server for server in servers if server.get("name") == "orbit"), None)
if orbit is None:
    raise SystemExit("FAIL: installed Codex plugin did not register an orbit MCP server")
transport = orbit.get("transport", {})
serialized = json.dumps(transport)
if "CLAUDE_PROJECT_DIR" in serialized:
    raise SystemExit("FAIL: Codex MCP server config references CLAUDE_PROJECT_DIR")
if transport.get("type") != "stdio":
    raise SystemExit(f"FAIL: expected stdio Orbit MCP transport: {transport!r}")
command = transport.get("command")
args = transport.get("args", [])
if not isinstance(command, str) or not command:
    raise SystemExit(f"FAIL: Orbit MCP transport command is missing: {transport!r}")
if not isinstance(args, list) or not all(isinstance(arg, str) and arg for arg in args):
    raise SystemExit(f"FAIL: Orbit MCP transport args are invalid: {transport!r}")
if command != "npx" or "mcp" not in args:
    raise SystemExit(f"FAIL: unexpected Orbit MCP transport: {transport!r}")
transport_path.write_text(json.dumps({"command": command, "args": args}), encoding="utf-8")
print("codex mcp => orbit server registered")
PY

echo "--- step 5: Codex fresh task prompt discovers Orbit skills ---"
CODEX_PROMPT_OUT="$TMPDIR_ROOT/codex-prompt.json"
if ! "${CODEX_ENV[@]}" codex -C "$WORKSPACE_DIR" debug prompt-input "Use Orbit to list tasks." \
     >"$CODEX_PROMPT_OUT" 2>"$TMPDIR_ROOT/codex-prompt.err"; then
  echo "FAIL: codex debug prompt-input exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/codex-prompt.err" >&2
  exit 1
fi
python3 - "$CODEX_PROMPT_OUT" <<'PY'
import json
import sys
text = json.dumps(json.load(open(sys.argv[1], encoding="utf-8")))
required = [
    "orbit:orbit",
    "orbit:orbit-task",
    "orbit:orbit-search",
    "orbit:orbit-knowledge",
    "orbit:orbit-graph",
]
missing = [name for name in required if name not in text]
if missing:
    raise SystemExit(f"FAIL: fresh Codex task prompt is missing Orbit skills: {missing}")
print("codex prompt-input => canonical Orbit skills discovered")
PY

echo "--- step 6: Codex plugin MCP command handles read-only tool call ---"
CODEX_RPC_IN="$TMPDIR_ROOT/codex-rpc-in.txt"
CODEX_RPC_OUT="$TMPDIR_ROOT/codex-rpc-out.txt"
CODEX_RPC_ERR="$TMPDIR_ROOT/codex-rpc-err.txt"
CODEX_MCP_ARGV=()
while IFS= read -r arg; do
  CODEX_MCP_ARGV+=("$arg")
done < <(python3 - "$CODEX_MCP_TRANSPORT" <<'PY'
import json
import sys

transport = json.load(open(sys.argv[1], encoding="utf-8"))
print(transport["command"])
for arg in transport["args"]:
    print(arg)
PY
)
if [[ "${#CODEX_MCP_ARGV[@]}" -eq 0 ]]; then
  echo "FAIL: Codex plugin MCP transport command was empty" >&2
  exit 1
fi

cat >"$CODEX_RPC_IN" <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-codex-plugin-install","version":"0.0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"orbit_task_list","arguments":{"status":"backlog","model":"codex"}}}
EOF

rc=0
{
  cat "$CODEX_RPC_IN"
  sleep 5
} | ( cd "$WORKSPACE_DIR" && run_with_timeout 120 "${CODEX_MCP_ARGV[@]}" ) \
     >"$CODEX_RPC_OUT" 2>"$CODEX_RPC_ERR" || rc=$?

if [[ "$rc" -ne 0 && "$rc" -ne 124 && "$rc" -ne 142 && "$rc" -ne 143 ]]; then
  echo "FAIL: Codex plugin MCP command exited with $rc" >&2
  echo "--- stderr ---" >&2
  cat "$CODEX_RPC_ERR" >&2
  echo "--- stdout ---" >&2
  cat "$CODEX_RPC_OUT" >&2
  exit 1
fi
if ! grep -q '"orbit_task_list"' "$CODEX_RPC_OUT"; then
  echo "FAIL: Codex plugin tools/list response did not include orbit_task_list" >&2
  echo "--- stdout ---" >&2
  cat "$CODEX_RPC_OUT" >&2
  echo "--- stderr ---" >&2
  cat "$CODEX_RPC_ERR" >&2
  exit 1
fi
assert_jsonrpc_tool_call_ok "$CODEX_RPC_OUT" 3

echo "PASS: agent plugin install chains serve MCP successfully (orbit $binary_version)"

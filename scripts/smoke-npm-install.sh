#!/usr/bin/env bash
# End-to-end smoke for the published npm install chain.
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
#
# Pass: exit 0. Fail: non-zero with the relevant stderr captured.
# Supported: macOS arm64 / x86_64, Linux x86_64 / arm64. Not Windows.
set -euo pipefail
NPM_PKG="@orbit-tools/cli"

assert_version_matches_tag() {
  local installed_version="$1"
  local expected_version="$2"

  if [[ "$installed_version" != "$expected_version" ]]; then
    echo "FAIL: installed $NPM_PKG version '$installed_version' does not match tag version '$expected_version'" >&2
    return 1
  fi
  echo "versioned smoke => $NPM_PKG@$installed_version matches tag"
}

run_version_assertion_test() {
  assert_version_matches_tag "0.14.0" "0.14.0"
  if assert_version_matches_tag "0.13.0" "0.14.0"; then
    echo "FAIL: version assertion accepted a mismatched version" >&2
    return 1
  fi
  echo "PASS: version assertion rejects a mismatched tag version"
}

if [[ "${1:-}" == "--dry-run-version-assertion" ]]; then
  run_version_assertion_test
  exit 0
fi
if [[ "$#" -ne 0 ]]; then
  echo "usage: $0 [--dry-run-version-assertion]" >&2
  exit 2
fi

require_bin() {
  local bin="$1"
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "smoke-npm-install: required binary '$bin' not on PATH" >&2
    exit 2
  fi
}

require_bin node
require_bin npx
require_bin npm

case "$(uname -s)" in
  Darwin|Linux) ;;
  *)
    echo "smoke-npm-install: unsupported OS '$(uname -s)' — supported: Darwin, Linux" >&2
    exit 2
    ;;
esac

TMPDIR_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

# A tag-triggered run must prove that npm has the package built from that tag.
# workflow_dispatch supplies SMOKE_NPM_INSTALL_TAG when a releaser reruns
# this workflow from the default branch against a published release tag.
tag_name="${SMOKE_NPM_INSTALL_TAG:-}"
if [[ -z "$tag_name" && "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  tag_name="${GITHUB_REF_NAME:-}"
fi
if [[ -z "$tag_name" && "${GITHUB_REF:-}" == refs/tags/* ]]; then
  tag_name="${GITHUB_REF#refs/tags/}"
fi
expected_tag_version=""
if [[ -n "$tag_name" ]]; then
  expected_tag_version="${tag_name#v}"
fi

# Cache npx installs inside the temp dir so we exercise a clean download.
export npm_config_cache="$TMPDIR_ROOT/npm-cache"
mkdir -p "$npm_config_cache"

# Sandbox HOME so `orbit init` writes to the temp tree instead of the
# runner/user's real ~/.orbit. Without this, repeat runs locally would
# accumulate state in the developer's global Orbit root.
SMOKE_HOME="$TMPDIR_ROOT/home"
mkdir -p "$SMOKE_HOME"
export HOME="$SMOKE_HOME"
# A caller-exported ORBIT_ROOT would bypass the HOME sandbox and write
# host identity / workspace state outside the throwaway tree.
unset ORBIT_ROOT

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
  echo "smoke-npm-install: need timeout, gtimeout, or perl on PATH" >&2
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

echo "--- step 1: npx -y $NPM_PKG@latest --version ---"
VERSION_OUT="$TMPDIR_ROOT/version.txt"
if ! npx -y "$NPM_PKG@latest" --version >"$VERSION_OUT" 2>"$TMPDIR_ROOT/version.err"; then
  echo "FAIL: npx -y $NPM_PKG@latest --version exited non-zero" >&2
  echo "--- stderr ---" >&2
  cat "$TMPDIR_ROOT/version.err" >&2
  exit 1
fi
binary_version="$(sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e 's/^orbit[[:space:]]*//' "$VERSION_OUT")"
if [[ -z "$binary_version" ]]; then
  echo "FAIL: orbit --version produced no output" >&2
  cat "$TMPDIR_ROOT/version.err" >&2
  exit 1
fi
echo "orbit --version => $binary_version"
if [[ -n "$expected_tag_version" ]]; then
  assert_version_matches_tag "$binary_version" "$expected_tag_version"
fi

# Also confirm the npm-registry version, for the smoke report.
npm_version="$(npm view "$NPM_PKG" version 2>/dev/null || echo '<unknown>')"
echo "$NPM_PKG@latest on npm => $npm_version"

echo "--- step 2: orbit init + workspace init ---"
# `orbit mcp serve` deliberately refuses to bootstrap a workspace (see
# OrbitRuntime::try_initialize_existing) — so without these two commands the
# MCP server attaches but serves an empty tool surface. Initializing first
# matches the documented binary-first installation flow.
# Fresh-host non-interactive init requires host identity (ORB-10721): a
# host name plus a 2-5 letter task prefix that is not ORB/ADR/L/F.
if ! npx -y "$NPM_PKG@latest" init --non-interactive \
     --host-name smoke-npm-install --task-prefix SMK \
     >"$TMPDIR_ROOT/init.out" 2>"$TMPDIR_ROOT/init.err"; then
  echo "FAIL: orbit init exited non-zero" >&2
  echo "--- stdout ---" >&2
  cat "$TMPDIR_ROOT/init.out" >&2
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
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-npm-install","version":"0.0.1"}}}
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

echo "PASS: npm install chain serves MCP successfully (orbit $binary_version)"

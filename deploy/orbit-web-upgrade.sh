#!/usr/bin/env bash
# orbit-web-upgrade — daily orbit binary upgrade on a box running orbit-web.
#
# Rebuilds agent-main, atomically swaps ~/.orbit/bin/orbit, restarts orbit-web,
# and health-checks the dashboard — rolling back to orbit.bak when the new
# binary does not come up healthy. No-ops when the freshly built binary is
# byte-identical to the installed one, so the daily timer never restarts the
# service pointlessly.
#
# Scheduled by deploy/orbit-web-upgrade.{service,timer}; see deploy/README.md.
# Runs unattended: every failure path either leaves the installed binary
# untouched (pre-swap aborts) or restores it from orbit.bak (post-swap).

set -euo pipefail

REPO="${ORBIT_REPO:-$HOME/workspace/constellation/codebases/orbit}"
BRANCH="${ORBIT_BRANCH:-agent-main}"
# ORBIT_BIN is overridable so the swap/rollback machinery can be drilled
# against a sandbox path without touching the real install.
BIN="${ORBIT_BIN:-$HOME/.orbit/bin/orbit}"
BAK="$BIN.bak"
WEB_URL="${ORBIT_WEB_URL:-http://127.0.0.1:7878}"
UNIT="orbit-web"
WORKSPACES_JSON="$HOME/.orbit/workspaces.json"

log() { echo "orbit-web-upgrade: $*"; }
die() { echo "orbit-web-upgrade: ABORT: $*" >&2; exit 1; }

# /healthz must answer "ok" and /api/workspaces must list registered ws_* ids.
# Retries for up to 30s so a just-restarted server gets time to bind.
health_ok() {
  local i
  for i in $(seq 1 30); do
    if [[ "$(curl -fsS --max-time 2 "$WEB_URL/healthz" 2>/dev/null)" == "ok" ]] &&
      curl -fsS --max-time 2 "$WEB_URL/api/workspaces" 2>/dev/null | grep -q '"id":"ws_'; then
      return 0
    fi
    sleep 1
  done
  return 1
}

active_workspace_roots() {
  python3 -c '
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
for ws in data["workspaces"]:
    if ws.get("status") == "active":
        print(ws["root"])
' "$WORKSPACES_JSON" 2>/dev/null
}

# Atomic install: never write into the live inode (the running orbit-web holds
# it executing — a plain `cp` over it fails with ETXTBSY). Rename replaces the
# directory entry instead.
install_binary() {
  local src="$1"
  install -m 0755 "$src" "$BIN.next"
  mv -f "$BIN.next" "$BIN"
}

main() {
  cd "$REPO"

  # ---- guards: pre-swap failures leave the installed binary untouched ------
  local head
  head=$(git symbolic-ref --short -q HEAD) || die "detached HEAD in $REPO"
  [[ "$head" == "$BRANCH" ]] || die "checkout is on '$head', not '$BRANCH' — refusing to build"
  [[ -z "$(git status --porcelain --untracked-files=no)" ]] ||
    die "working tree in $REPO is dirty — refusing to pull/build"
  git pull --ff-only --quiet || die "git pull --ff-only failed"
  cargo build --release --quiet || die "cargo build --release failed"

  local new="$REPO/target/release/orbit"
  [[ -x "$new" ]] || die "build produced no binary at $new"

  # ---- no-op path: installed binary already matches the agent-main build ---
  if cmp -s "$new" "$BIN"; then
    log "installed binary already matches agent-main build ($("$BIN" --version)) — no-op"
    exit 0
  fi

  local cur_ver new_ver
  cur_ver=$("$BIN" --version 2>/dev/null || echo "unknown")
  new_ver=$("$new" --version)

  # ---- migration gate -------------------------------------------------------
  # Orbit auto-applies pending migrations on workspace open by design (see
  # `orbit migrate --help`), so *pending* migrations do not block the swap —
  # they are logged. A `migrate --dry-run` that errors outright means the new
  # binary cannot even inspect a workspace: abort before touching anything.
  if [[ "$cur_ver" != "$new_ver" ]]; then
    local ws_root out
    while IFS= read -r ws_root; do
      [[ -d "$ws_root/.orbit" ]] || continue
      if out=$("$new" migrate --dry-run --json --root "$ws_root/.orbit" 2>/dev/null); then
        :
      elif echo "$out" | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null; then
        # Nonzero exit but valid JSON = migrations pending, inspection worked.
        log "pending migrations in $ws_root (auto-apply on open): $(echo "$out" | tr -d '\n' | head -c 300)"
      else
        die "migrate --dry-run failed in $ws_root with $new_ver: $("$new" migrate --dry-run --root "$ws_root/.orbit" 2>&1 | tail -5 | tr '\n' ' ')"
      fi
    done < <(active_workspace_roots)
  fi

  # ---- mid-flight check (cheap, fail-soft) ----------------------------------
  # Don't restart under an active pipeline/ship run; defer to tomorrow's tick.
  # Worker CLI runs don't go through orbit-web, so only job runs matter here.
  local ws_root
  while IFS= read -r ws_root; do
    [[ -d "$ws_root/.orbit" ]] || continue
    if "$BIN" run history --json --limit 20 --root "$ws_root/.orbit" 2>/dev/null |
      grep -Eq '"status" *: *"(running|pending)"'; then
      log "active job run in $ws_root — deferring upgrade to the next timer tick"
      exit 0
    fi
  done < <(active_workspace_roots)

  # ---- swap -----------------------------------------------------------------
  cp -p "$BIN" "$BAK" || die "could not back up $BIN to $BAK"
  install_binary "$new"
  systemctl --user restart "$UNIT" || log "systemctl restart $UNIT reported failure — verifying health anyway"

  if health_ok; then
    log "upgraded: '$cur_ver' -> '$new_ver'; $UNIT healthy"
    exit 0
  fi

  # ---- rollback --------------------------------------------------------------
  log "health check FAILED after swap to '$new_ver' — rolling back to orbit.bak"
  install_binary "$BAK"
  systemctl --user restart "$UNIT" || true
  if health_ok; then
    log "rollback succeeded: $UNIT healthy again on '$cur_ver'"
  else
    log "rollback did NOT restore health — $UNIT unhealthy, manual intervention required"
  fi
  # Surface loudly: friction record in polaris (fail-soft — the rolled-back
  # binary is doing the writing) plus nonzero exit so the unit lands in
  # `systemctl --user list-units --failed`.
  "$BIN" friction add --model claude --tag tooling \
    --body "orbit-web-upgrade: swap to '$new_ver' failed the post-restart health check on $(hostname); rolled back to '$cur_ver' (orbit.bak). See: journalctl --user -t orbit-web-upgrade" \
    --root "$HOME/workspace/constellation/knowledgebase/polaris/.orbit" 2>/dev/null ||
    log "could not record friction (non-fatal)"
  die "upgrade to '$new_ver' failed health check; rolled back to '$cur_ver'"
}

# Everything above only defines functions/variables; the single call below is
# the last line the shell needs from this file. That keeps the run safe even
# though `git pull` may rewrite this very script mid-execution (bash reads
# scripts incrementally).
main "$@"

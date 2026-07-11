# Deploy units

Host-level scheduling artifacts for orbit. Nothing here is required to *use*
orbit — these are optional units for machines that run unattended orbit work.

## orbit-ship-sweep (systemd user timer)

Periodically runs `orbit run ship-sweep`, which walks every workspace in
`~/.orbit/workspaces.json` and submits a `ship` run (PR mode) where **all** of
the following hold:

1. the workspace opted in with `[workflow] auto_ship = true` in its
   `.orbit/config.toml` (default is off — the sweep never ships a workspace
   nobody blessed);
2. it has at least one `backlog` task whose dependencies are satisfied;
3. no `task_auto_pipeline` run is already pending/running there.

Everything else is reported as skipped. Per-workspace failures are isolated;
the unit exits non-zero only if a workspace errored, so failures surface in
`systemctl --user list-units --failed`.

A failed sweep retries after 5s (`Restart=on-failure`, `RestartSec=5`), capped
at 5 starts per 200s (`StartLimitIntervalSec`/`StartLimitBurst`) so a
persistent failure trips the start limit instead of thrashing. `Restart=` on a
`Type=oneshot` unit requires systemd >= 254 — on older hosts drop the
`Restart=`/`RestartSec=` lines; the timer still retries every 20 minutes.

The unit sets `KillMode=process` [ORB-10038]: the sweep dispatches pipeline
runs by spawning **detached** worker processes and exits immediately, and the
default `KillMode=control-group` kills the whole cgroup when the oneshot
deactivates — reaping the just-spawned workers (observed as
`task_auto_pipeline` runs flipping to `interrupted` with
`process_not_found` seconds after "Finished orbit-ship-sweep.service").
`orbit-qa-sweep.service` deliberately keeps the default: qa-sweep runs its
checks inline and finishes its ledger run before exiting, so cgroup cleanup
is correct there.

### Install (per host, e.g. dk-server-1)

```sh
mkdir -p ~/.config/systemd/user
cp deploy/orbit-ship-sweep.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now orbit-ship-sweep.timer
```

### Verify

```sh
orbit run ship-sweep --dry-run --json   # what would ship right now, no submits
systemctl --user list-timers orbit-ship-sweep.timer
journalctl --user -u orbit-ship-sweep -n 50
```

### Opt a workspace in

```toml
# <workspace>/.orbit/config.toml
[workflow]
base_branch = "agent-main"   # repo's ship base, if not "main"
auto_ship = true
```

## orbit-qa-sweep (systemd user timer)

Periodically runs `orbit run qa-sweep` [ORB-10039]: the trailing QA pass over
direct-push workspaces (design D4 — writes to `agent-main` stay fast;
validation happens on a lag). Per workspace listed in the `[qa]` section of
the **global** `~/.orbit/config.toml`:

1. skip unless the live checkout is on the expected branch and its HEAD moved
   past the per-workspace last-validated watermark
   (`~/.orbit/state/qa-sweep.json`);
2. run the configured checks (`sh -c`, from the workspace root; per-check
   timeout, muted checks skipped);
3. file a fingerprint-deduped orbit task per distinct failure (tags
   `qa-sweep` + `fp-<hash>`; an open task with the same fingerprint suppresses
   refiling);
4. advance the watermark only when every non-muted check passed.

Each validating pass is recorded in the workspace's run ledger under job id
`qa_sweep` (`orbit run history -j qa_sweep`, `orbit run show <run_id>`). The
unit exits non-zero only on workspace *errors* (misconfig, git/probe
failures) — a red check files a task and exits 0, so `--failed` means the
sweep itself is broken, not the code it checks.

Config lives in the **global** config.toml only (workspace `config.toml`
files are rewritten by task-mutation commands and are replace-not-merge):

```toml
# ~/.orbit/config.toml
[qa]
default_priority = "medium"    # priority of auto-filed QA tasks
task_status = "backlog"        # or "proposed" to require human approval

[[qa.workspace]]
name = "polaris"               # registry name (orbit workspace list)
branch = "agent-main"          # defaults to the registered base_branch

[[qa.workspace.check]]
name = "frontmatter"
command = "python3 scripts/check_frontmatter.py"

[[qa.workspace.check]]
name = "lint"
command = "make lint"
mute = true                    # flaky: keep the definition, skip the check
timeout_minutes = 10           # default 30
```

## orbit-web-upgrade (systemd user timer)

Daily `orbit-web-upgrade.sh` run: rebuild `agent-main` in the orbit checkout,
atomically swap `~/.orbit/bin/orbit` (previous binary kept as `orbit.bak`),
restart `orbit-web`, and health-check `/healthz` + `/api/workspaces`. This
automates the manual upgrade runbook in `docs/OPERATIONS.md` so the dashboard
never drifts behind `agent-main`.

Safety properties, in order:

1. **No-op** when the freshly built binary is byte-identical to the installed
   one — no swap, no restart, no daily service blip.
2. **Pre-swap aborts leave everything untouched**: wrong branch, dirty tree,
   failed `git pull --ff-only`, failed `cargo build --release`, or an
   `orbit migrate --dry-run` that errors under the new binary (merely *pending*
   migrations don't block — orbit auto-applies them on workspace open).
3. **Deferral** instead of restart when any registered workspace has a
   pending/running job run whose worker process is genuinely alive
   (`orbit run history` + a pid/cmdline liveness probe). Stale orphans —
   e.g. `pending` runs stranded by a reboot — no longer defer the swap;
   orbit reconciles them to `interrupted` on workspace open [ORB-10070].
4. **Rollback**: if `/healthz` + `/api/workspaces` don't come back healthy
   within the post-restart retry window (30 attempts), the script reinstalls `orbit.bak`, restarts
   again, records a friction in polaris, and exits nonzero so the failure
   shows in `systemctl --user list-units --failed`.

It's a systemd user timer (not an orbit routine) deliberately: the job
replaces and restarts the very binary the orbit scheduler runs on, so it
lives outside orbit — like its siblings above, it still works when orbit is
the thing that's broken.

### Install (per host, e.g. dk-server-1)

```sh
<<<<<<< HEAD
cp deploy/orbit-web-upgrade.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now orbit-web-upgrade.timer
```

The service `ExecStart` points at this script inside the checkout
(`~/workspace/constellation/codebases/orbit/deploy/orbit-web-upgrade.sh`), so
script fixes land with a normal `git pull` — only `.service`/`.timer` edits
need a re-copy + `daemon-reload`.

### Inspect / disable

```sh
systemctl --user list-timers orbit-web-upgrade.timer     # next/last fire
journalctl --user -t orbit-web-upgrade -n 50             # last run's output
systemctl --user start orbit-web-upgrade.service         # run once now
systemctl --user disable --now orbit-web-upgrade.timer   # stop the schedule
=======
mkdir -p ~/.config/systemd/user
cp deploy/orbit-qa-sweep.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now orbit-qa-sweep.timer
```

### Verify

```sh
orbit run qa-sweep --dry-run --json     # what would be validated, runs nothing
systemctl --user list-timers orbit-qa-sweep.timer
journalctl --user -u orbit-qa-sweep -n 50
>>>>>>> 1ec6d2d3 (feat(qa-sweep): scheduled QA routine trailing agent-main)
```

## orbit-web (systemd user service)

Runs `orbit web serve --global --no-open`: the always-on dashboard over every
registered workspace, bound to loopback (`127.0.0.1:7878`). Crashes restart
after 5s (`Restart=on-failure`, `RestartSec=5`), capped at 5 starts per 200s
(`StartLimitIntervalSec`/`StartLimitBurst`).

Liveness: `GET /healthz` answers `200 ok` while the process is up. Readiness /
self-diagnosis: `GET /healthz?detailed=true` runs cheap per-workspace checks
(SQLite writable, graph index readable, log sink writable) and returns `503`
with a per-check JSON body when any check fails — point uptime monitoring at
the detailed form.

### Install (per host, e.g. dk-server-1)

```sh
mkdir -p ~/.config/systemd/user
cp deploy/orbit-web.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now orbit-web.service
```

After swapping the `orbit` binary, restart the unit:
`systemctl --user restart orbit-web`.

### Verify

```sh
systemctl --user status orbit-web
curl -s localhost:7878/healthz
curl -s 'localhost:7878/healthz?detailed=true' | jq .
```

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

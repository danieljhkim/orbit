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

---
title: "Remote Access — Design"
owner: claude
last_updated: 2026-07-05
status: Accepted
feature: remote-access
doc_role: design
type: design
summary: "The two shipped surfaces — global multi-workspace serve (the only mode since ORB-10029) and SSH-tunnel connect — their state model, and how they compose."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**", "crates/orbit-cli/src/command/web.rs"]
related_features: [remote-access, user-interface]
related_artifacts: [ORB-00029, ORB-00030, ORB-00360, ORB-10029, ADR-0200, ADR-0201]
---

# Remote Access — Design

This document specifies the two shipped surfaces of remote access — global multi-workspace serve and SSH-tunnel connect — the workspace-keyed state they share, the CLI dispatch that lets them run from anywhere, and the security floor they inherit. The read-only-viewing boundary and the decision not to build a writable cross-machine registry are covered in [4_decisions.md](./4_decisions.md); the dashboard's per-request UI machinery is owned by [user-interface](../user-interface/1_overview.md).

---

## 1. Global multi-workspace serve

`orbit web serve` always serves in global mode: [`build_state`](../../../crates/orbit-dashboard/src/lib.rs) unconditionally enumerates `~/.orbit/workspaces.json` via `orbit_core::workspace_registry` (`global_orbit_dir` → `load_registry` → `validate_workspaces`) and builds a `DashboardState::global`. Each registry entry becomes a `WsEntry { id, name, repo_root, orbit_dir, active }`, where `active` mirrors registry status — stale-path workspaces flip to inactive. `default_workspace_selection` picks the dropdown's default selection when a request omits `?workspace=`, in priority order: the top-level `--root <path>` flag (`root_override`), if given and it resolves to a registered/active workspace via `default_workspace_for_cwd`; else the longest active repo-root prefix of the process cwd (also `default_workspace_for_cwd`); else `None` (the frontend opens the aggregate "All workspaces" view). A given `--root` that matches no workspace resolves straight to `None` — it never falls back to the cwd-based default, is never an error, and never auto-registers. The `--root` path matters specifically for `orbit web connect` (§3): the remote `orbit web serve` it launches runs non-interactively over `ssh` with cwd set to the SSH user's home directory, so `--root` is the only signal that can hint which workspace to preselect there.

[ORB-10029] made this the only mode: single-workspace serving used to be the default when `orbit web serve` ran inside a workspace (opening that workspace's runtime via `OrbitRuntime::try_initialize_existing` and wrapping it in `DashboardState::single`), reachable only via the then-opt-in `--global` flag otherwise. `--global` is now a deprecated no-op, kept parsing only because `orbit web connect` (§3) unconditionally forwards it to the remote `orbit web serve`. `DashboardState::single` and the `serve(runtime, args)` entry point are retained for callers that already hold a built `OrbitRuntime` and want it embedded directly, and for the dashboard's handler tests (an in-memory runtime needs no lazy registry lookup) — but `orbit web serve` no longer reaches either.

## 2. Workspace-keyed state and the `Ws` extractor

[`DashboardState`](../../../crates/orbit-dashboard/src/state.rs) holds a set of `WsEntry` plus a `Mutex<HashMap<String, Arc<OrbitRuntime>>>` runtime cache. `runtime_for(id)`:

1. rejects an unknown id (`404`) and an inactive id (`400`);
2. returns the cached runtime if present;
3. otherwise builds `OrbitRuntime::from_roots(global_root, entry.orbit_dir).with_actor(human)` **outside** the cache lock, then inserts cache-first so a concurrent build is harmless.

The `Ws(pub(crate) Arc<OrbitRuntime>)` extractor implements `FromRequestParts<DashboardState>`: it reads `?workspace=<id>` (percent-decoded; empty is treated as absent), else the configured default, else rejects with a structured JSON `{ "error": ... }`. Handlers changed only their signature line — `State(runtime): State<Arc<OrbitRuntime>>` → `Ws(runtime): Ws` — so 46 handler bodies were untouched. This "query-param choke point + one-line signature swap" was chosen over workspace-prefixed route paths; the rationale is [user-interface ADR-00030](../user-interface/4_decisions.md).

Two aggregate endpoints expose the machine-wide surface:

- `GET /api/workspaces` — the servable workspaces `{ id, name, root, status, is_default }`.
- `GET /api/tasks/all` — iterates active workspaces, opens each runtime, and tags every task with `workspace_id` / `workspace_name`; an unopenable workspace is skipped, not fatal.

The frontend adds a header workspace selector and an "All workspaces" aggregate task view. `common.js` wraps every fetch in `withWorkspace()` (appending `?workspace=` unless already present); `app.js` uses `/api/tasks/all` when more than one workspace exists and none is selected.

## 3. SSH-tunnel connect

[`connect`](../../../crates/orbit-dashboard/src/connect.rs) automates the manual `ssh -L 7878:localhost:7878 <host> "orbit web serve --no-open"` dance and nothing more. Given `orbit web connect <ssh-host>`:

1. **Local port.** `select_local_port` prefers the conventional `7878`, falling back to an OS-assigned ephemeral port if it is busy; an explicit `--port` is honored or fails loudly.
2. **Remote command.** `remote_serve_command` builds `orbit web serve --no-open --port <remote_port> [--global] [--root <p>]`. `--no-open` is always present (the remote must never open a browser); `--root` is shell-quoted; `--global` is forwarded when set — a no-op against a post-[ORB-10029] remote binary (global is always on), kept so `connect` still works against an older remote that still gates on the flag.
3. **Tunnel.** `build_ssh_args` produces `ssh -tt -o ExitOnForwardFailure=yes -L <local>:localhost:<remote> <host> <remote-command>`. `-tt` forces a pty so the remote serve receives SIGHUP when the tunnel drops; `ExitOnForwardFailure=yes` fails fast rather than running the remote with no working forward; stdin is null so Ctrl-C reaches *us*.
4. **Readiness.** `wait_until_ready` polls `GET /healthz` over the forwarded port until it answers `200`, the `ssh` child exits early (classified into an actionable error — `127` = orbit not on remote PATH, `255` = ssh connect failure), or a 30s timeout elapses.
5. **Teardown.** `SshTunnel` is an RAII owner of the `ssh` child; on Ctrl-C / SIGTERM / remote exit, `Drop` sends SIGTERM then SIGKILL after a grace period. Closing `ssh` drops the connection, SIGHUP reaps the remote serve — no orphan.

## 4. CLI dispatch from anywhere

Both surfaces must run outside a workspace, so [`main.rs`](../../../crates/orbit-cli/src/main.rs) dispatches the entire `Web` command *before* the CLI's eager `OrbitRuntime::initialize_with_root_override` (which fails when there is no current workspace): `serve` calls `serve_from_env`, `connect` calls `connect`. Missing either arm sends the command into eager init and breaks it outside a workspace — the failure mode [ORB-00029]'s follow-up fixed for `connect`.

## 5. Security floor (inherited, unchanged)

Neither surface touches the loopback-only bind guard from [ORB-00360]: `check_bindable_host` refuses any non-loopback host, so the dashboard never listens on a routable interface. Global mode broadens *data* exposure only on the same machine; connect's wire is SSH. There is no token, no ACL, no session — auth is delegated entirely to SSH and to local machine access. See [4_decisions.md ADR-0201](./4_decisions.md).

---

## 6. Concerns & Honest Limitations

- **Viewing is not sync.** The defining limitation: no offline path, no cross-machine write or merge, and the aggregate is per-machine. Restated from [1_overview.md §2.4](./1_overview.md) because it is the feature's load-bearing tradeoff ([ADR-0200](./4_decisions.md)).
- **Remote must be reachable and provisioned.** `connect` needs SSH reachability *and* `orbit` on the remote's non-interactive PATH (the `127` exit path exists precisely because this is the common misconfiguration). No browser-only or tokened-URL access exists.
- **Aggregate reopens stores per request.** `GET /api/tasks/all` opens each workspace's store on every call — there is no cross-workspace caching of task lists yet.
- **Unauthenticated on the wire it does reach.** On the loopback interface (local, or the forwarded port), the API is unauthenticated by design; anyone with local access or a foothold on the forwarded port has full dashboard access. The mitigation is the bind guard + SSH, not in-app auth.
- **Port selection is racy (TOCTOU).** `select_local_port` probes then hands the port to `ssh`; another process can claim it in between. Acceptable for a developer convenience — `ssh -L` fails loudly if so.

---

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>` and later forwarded `--global` to the remote serve.
- [ORB-00030] — Workspace-keyed state, `Ws` extractor, global serve, aggregate endpoints, frontend selector.
- [ORB-00360] — Loopback-only bind guard and stored-XSS fix.
- [ORB-10029] — Made global mode the only mode for `orbit web serve`; `--global` is now a deprecated no-op kept for `connect` passthrough compatibility.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

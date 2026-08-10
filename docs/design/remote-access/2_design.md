---
title: "Remote Access — Design"
owner: claude
last_updated: 2026-08-10
status: Accepted
feature: remote-access
doc_role: design
type: design
summary: "The two shipped surfaces — global multi-workspace serve (the only mode since ORB-10029) and SSH-tunnel connect (attach-if-already-running since ORB-10708) — their state model, the dashboard task-list endpoint contract, and how they compose."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**", "crates/orbit-remote/src/runtime.rs", "crates/orbit-remote/src/workspace_registry.rs", "crates/orbit-cli/src/command/web.rs", "crates/orbit-cli/src/command/operation.rs"]
related_features: [remote-access, user-interface, host-registry]
related_artifacts: [ORB-00029, ORB-00030, ORB-00360, ORB-10029, ORB-10200, ORB-10294, ORB-10310, ORB-10319, ORB-10400, ORB-10708, ADR-0200, ADR-0201, ADR-0234, ADR-0353]
---

# Remote Access — Design

This document specifies the two shipped surfaces of remote access — global multi-workspace serve and SSH-tunnel connect — the workspace-keyed state they share, the CLI dispatch that lets them run from anywhere, and the security floor they inherit. The read-only-viewing boundary and the decision not to build a writable cross-machine registry are covered in [4_decisions.md](./4_decisions.md); the dashboard's per-request UI machinery is owned by [user-interface](../user-interface/1_overview.md).

---

## 1. Global multi-workspace serve

`orbit web serve` always serves in global mode: [`build_state`](../../../crates/orbit-dashboard/src/lib.rs) enumerates `~/.orbit/workspaces.json` via `orbit_remote::workspace_registry` (`global_orbit_dir` → `registry_path` → `load_registry_from` → `validate_workspaces`) and builds a registry-backed `DashboardState::from_registry`, capturing a `RegistrySource { registry_path, root_override, cwd }` so the servable set can be reloaded later (§2.1). Each registry entry becomes a `WsEntry { id, name, repo_root, orbit_dir, active }`, where `active` mirrors registry status — stale-path workspaces flip to inactive. Runtime construction delegates to `orbit_remote::runtime::RemoteRuntimeFactory`, so catalog binding and checkout/profile resolution have the same owner as MCP and routine composition. `default_workspace_selection` picks the dropdown's default selection when a request omits `?workspace=`, in priority order: the top-level `--root <path>` flag (`root_override`), if given and it resolves to a registered/active workspace via `default_workspace_for_cwd`; else the longest active repo-root prefix of the process cwd (also `default_workspace_for_cwd`); else `None` (the frontend opens the aggregate "All workspaces" view). A given `--root` that matches no workspace resolves straight to `None` — it never falls back to the cwd-based default, is never an error, and never auto-registers. The `--root` path matters specifically for `orbit web connect` (§3): the remote `orbit web serve` it launches runs non-interactively over `ssh` with cwd set to the SSH user's home directory, so `--root` is the only signal that can hint which workspace to preselect there. [ORB-10319]

[ORB-10029] made this the only mode: single-workspace serving used to be the default when `orbit web serve` ran inside a workspace (opening that workspace's runtime via `OrbitRuntime::try_initialize_existing` and wrapping it in `DashboardState::single`), reachable only via the then-opt-in `--global` flag otherwise. `--global` is now a deprecated no-op, kept parsing only because `orbit web connect` (§3) unconditionally forwards it to the remote `orbit web serve`. `DashboardState::single` and the `serve(runtime, args)` entry point are retained for callers that already hold a built `OrbitRuntime` and want it embedded directly, and for the dashboard's handler tests (an in-memory runtime needs no lazy registry lookup) — but `orbit web serve` no longer reaches either.

## 2. Workspace-keyed state and the `Ws` extractor

[`DashboardState`](../../../crates/orbit-dashboard/src/state.rs) holds a `Mutex<Arc<Snapshot>>` (the servable `WsEntry` set plus the default selection, stamped with a monotonic **generation**) and a `Mutex<HashMap<String, CachedRuntime>>` runtime cache, where each `CachedRuntime` records both the `repo_root`/`orbit_dir` binding **and the generation** it was built for. The cache is *non-authoritative*: a pinned `Snapshot` is the sole authority for a workspace's binding, and every cache read and publication is validated against it. Runtime resolution (`resolve_runtime(snapshot, id)`):

1. resolves the binding from the **pinned** snapshot, rejecting an unknown id (`404`) and an inactive id (`400`), holding no lock;
2. returns the cached runtime only if its binding still matches that snapshot (a rebound workspace, or a stale entry left by an older-snapshot publication, is a cache miss);
3. otherwise builds `OrbitRuntime::from_roots(global_root, entry.orbit_dir).with_actor(human)` **outside** the cache lock, then publishes under it with generation discipline: an identical binding is idempotent (first build wins); a build whose generation is **older** than a differing cached binding is never published — it is returned only to its own pinned request, so an older-snapshot build can never overwrite or be surfaced as the current runtime. `open_runtimes` likewise joins the cache to the pinned snapshot by exact binding, not by id, so a stale entry is never reported or tagged as the wrong checkout.

The `Ws(pub(crate) Arc<OrbitRuntime>)` extractor implements `FromRequestParts<DashboardState>`: it calls `pin()` (refresh + pin one snapshot, §2.1), then reads `?workspace=<id>` (percent-decoded; empty is treated as absent), else the pinned default, else rejects with a structured JSON `{ "error": ... }` — resolving the runtime against the same pinned generation. Handlers changed only their signature line — `State(runtime): State<Arc<OrbitRuntime>>` → `Ws(runtime): Ws` — so 46 handler bodies were untouched. This "query-param choke point + one-line signature swap" was chosen over workspace-prefixed route paths; the rationale is [user-interface ADR-00030](../user-interface/4_decisions.md).

### 2.1 Live registry refresh ([ORB-10294])

The servable set is not frozen at startup. A registry-backed `DashboardState` reloads `~/.orbit/workspaces.json` at each request boundary via `DashboardState::refresh()`, so a native `orbit workspace init/remove` or a changed root/orbit-dir binding is honored **without restarting the server** — the restart this replaced was a Build Week cleanup papercut. Every request refreshes and then **pins one snapshot generation** with `DashboardState::pin()` (returning a `Pinned` view), and derives everything from it — default selection, entry metadata, runtime resolution, and the open-runtime set. The `Ws` extractor pins for all 46 routed handlers; the three aggregate/host handlers that read the entry set directly — `GET /api/workspaces`, `GET /api/tasks/all`, and detailed `GET /healthz` — each pin once per response, so a concurrent add/remove/rebind is observed as one coherent old-or-new generation and never splices old entry metadata onto a runtime resolved from a newer binding. States built via `DashboardState::single`/`::global` carry no `RegistrySource`, so `refresh()` is a no-op there (handler tests keep their fixed entries).

`refresh()` guarantees:

- **Atomic swap.** It reloads into a fresh, generation-stamped `Snapshot`, then replaces the whole `Arc<Snapshot>` in one assignment under the snapshot mutex; a concurrent reader sees the old view or the new one, never a half-applied update. A dedicated refresh mutex serializes refreshes so the swap and the runtime eviction that follows are one step relative to other refreshes.
- **Runtime eviction, off the lock.** After the swap it drops cache entries whose workspace is gone, inactive, or rebound (binding mismatch), leaving every other workspace's live runtime untouched. Runtimes are only ever *built* lazily in `resolve_runtime`, never under the registry/cache mutation lock; the generation stamp then guarantees an in-flight older-snapshot build cannot re-publish over the newer binding after eviction.
- **Keep-last-valid.** A malformed or partially-written registry read leaves the current snapshot in place and emits a credential-safe diagnostic — the registry path and Orbit's own error, never the file contents, so a tokenized `git_remote` cannot leak into logs. The initial `from_registry` load is the exception: a malformed registry **at startup** is fatal, exactly as before refresh existed.
- **No auto-delete.** A path that disappears after startup flips its entry to inactive (`validate_workspaces`) and the workspace is rejected with `400` on routing, but the operator's registry record is never rewritten or removed. Operator recovery: restore the checkout (or `orbit workspace` re-point it) and the next request re-activates it — no restart, no manual cache flush.

The rationale for request-driven reload over a filesystem watcher daemon, and for eventual-consistency under concurrent mutation, is [ADR-0234](./4_decisions.md).

Two aggregate endpoints expose the machine-wide surface:

- `GET /api/workspaces` — the servable workspaces `{ id, name, root, status, is_default }`.
- `GET /api/tasks/all` — iterates active workspaces, opens each runtime, and tags every task with `workspace_id` / `workspace_name`; an unopenable workspace is skipped, not fatal.

The frontend adds a header workspace selector and an "All workspaces" aggregate task view. `common.js` wraps every fetch in `withWorkspace()` (appending `?workspace=` unless already present); `app.js` uses `/api/tasks/all` when more than one workspace exists and none is selected.

### 2.2 `GET /api/tasks` — server-side filters and page metadata ([ORB-10400])

The workspace-scoped task list is the endpoint programmatic clients (notably Bridge's `orbit_task_list`) read, so it carries the native `orbit task list` filters rather than making each client re-derive them:

| Query parameter | Semantics |
|---|---|
| `status` | Repeatable and/or comma-separated. **OR** across values; omitted is status-neutral (every lifecycle status, `done`/`archived` included — [ORB-10310]). |
| `tag` (alias `tags`) | Repeatable and/or comma-separated. **AND** across values, normalized by `normalize_task_tags`/`task_matches_tags` — the same predicate `orbit task list --tag` uses. Only `,` separates values, so a colon is ordinary tag content and `auto-task:qa-sweep` matches that whole tag. |
| `type` (alias `task_type`) | A single `feature`/`bug`/`refactor`/`chore`. |
| `limit` | Positive integer; defaults to `DEFAULT_TASK_LIST_LIMIT` (50). |

Unknown keys are ignored — the `?workspace=<id>` selector consumed by the `Ws` extractor shares this query string — but an *unparseable* value for a recognized key is a `400`, never a silently dropped filter that would return an unfiltered page a client cannot distinguish from a real match set.

The response is a page envelope rather than a bare array:

```json
{ "items": [ /* task objects, newest first */ ], "total": 137, "limit": 50, "truncated": true }
```

Two properties make this usable as a provider contract:

- **Every predicate is applied before the limit.** `items` holds the newest *matching* tasks, so a match older than the newest `limit` unfiltered tasks is still reachable by passing its filters. Before [ORB-10400] the handler truncated the unfiltered set to 50 and offered no filters at all, so any client filtering the response client-side simply lost every older match — Bridge saw one `auto-task:qa-sweep` task where the CLI saw seven, and 13 `proposed` where the CLI saw 14.
- **`total` and `truncated` distinguish empty from truncated.** `total` is the pre-limit match count and `truncated` is `total > items.len()`, so `{"items": [], "total": 0}` is a genuinely empty result while a partial window announces itself.

Consumers accept either shape (`items` when present, else the array) — the `/api/tasks/all` aggregate is still a bare array, takes no filters, and is bounded by the same default limit. `common.js` exposes `listItems(payload)` for the frontend; Bridge's `_as_items` already had the same tolerance.

## 3. SSH-tunnel connect

[`connect`](../../../crates/orbit-dashboard/src/connect.rs) automates the manual `ssh -L 7878:localhost:7878 <host> "orbit web serve --no-open"` dance and nothing more. Given `orbit web connect <ssh-host>`:

1. **Local port.** `select_local_port` prefers the conventional `7878`, falling back to an OS-assigned ephemeral port if it is busy; an explicit `--port` is honored or fails loudly.
2. **Attach probe ([ORB-10708], [ADR-0353]).** Before spawning anything, `probe_attach` opens a bare forward with no remote command (`build_probe_ssh_args`: `ssh -N -o ExitOnForwardFailure=yes -L <local>:localhost:<remote> <host>`) and polls `GET /healthz` over it (`poll_until_ready`) for `ATTACH_PROBE_TIMEOUT` (5s). If something already answers `200` — another session's tunnel, or a long-lived remote listener — that bare forward *is* the session's tunnel from here on: no remote command is ever sent, so the pre-existing remote process is never touched by this invocation, on connect or on later disconnect. If nothing answers before the probe timeout, the probe forward is torn down and `connect` falls through to spawn mode (steps 3–4) unchanged. An early `ssh` exit during the probe (bad host, auth failure) is not swallowed — it is classified and surfaced immediately, the same as in spawn mode.
3. **Remote command (spawn mode only).** `remote_serve_command` builds `orbit web serve --no-open --port <remote_port> [--global] [--root <p>]`. `--no-open` is always present (the remote must never open a browser); `--root` is shell-quoted; `--global` is forwarded when set — a no-op against a post-[ORB-10029] remote binary (global is always on), kept so `connect` still works against an older remote that still gates on the flag.
4. **Tunnel (spawn mode only).** `build_ssh_args` produces `ssh -tt -o ExitOnForwardFailure=yes -L <local>:localhost:<remote> <host> <remote-command>`. `-tt` forces a pty so the remote serve receives SIGHUP when the tunnel drops; `ExitOnForwardFailure=yes` fails fast rather than running the remote with no working forward; stdin is null so Ctrl-C reaches *us*. `wait_until_ready` (built on the same `poll_until_ready` the attach probe uses) polls `/healthz` until it answers `200`, the `ssh` child exits early (classified into an actionable error — `127` = orbit not on remote PATH, `255` = ssh connect failure), or the full 30s `READINESS_TIMEOUT` elapses.
5. **Teardown.** `SshTunnel` is an RAII owner of whichever `ssh` child this invocation is holding (the attach probe's bare forward, or the spawn-mode tunnel); on Ctrl-C / SIGTERM / remote exit, `Drop` sends SIGTERM then SIGKILL after a grace period. In spawn mode, closing `ssh` drops the connection and SIGHUP reaps the remote serve — no orphan. In attach mode there is no remote command tied to the session, so closing `ssh` only closes the forward — the pre-existing remote process it attached to keeps running.

## 4. CLI dispatch from anywhere

Both surfaces must run outside a workspace. The exhaustive [`Commands::operation`](../../../crates/orbit-cli/src/command/operation.rs) declaration therefore marks the entire `Web` command `RuntimeNeed::Forbidden` and dispatches `serve` to `serve_from_env` and `connect` to `connect`. [`main.rs`](../../../crates/orbit-cli/src/main.rs) derives its pre-bootstrap path from that operation data instead of re-enumerating web subcommands. Omitting or changing the operation arm sends the command into eager runtime initialization and breaks it outside a workspace — the failure mode [ORB-00029]'s follow-up fixed for `connect`; [ORB-10200] makes the policy compiler-enforced alongside audit and output concerns.

## 5. Security floor (inherited, unchanged)

Neither surface touches the loopback-only bind guard from [ORB-00360]: `check_bindable_host` refuses any non-loopback host, so the dashboard never listens on a routable interface. Global mode broadens *data* exposure only on the same machine; connect's wire is SSH. There is no token, no ACL, no session — auth is delegated entirely to SSH and to local machine access. See [4_decisions.md ADR-0201](./4_decisions.md).

---

## 6. Concerns & Honest Limitations

- **Viewing is not sync.** The defining limitation: no offline path, no cross-machine write or merge, and the aggregate is per-machine. Restated from [1_overview.md §2.4](./1_overview.md) because it is the feature's load-bearing tradeoff ([ADR-0200](./4_decisions.md)).
- **Remote must be reachable and provisioned.** `connect` needs SSH reachability *and* `orbit` on the remote's non-interactive PATH (the `127` exit path exists precisely because this is the common misconfiguration). No browser-only or tokened-URL access exists.
- **Aggregate reopens stores per request.** `GET /api/tasks/all` opens each workspace's store on every call — there is no cross-workspace caching of task lists yet.
- **Unauthenticated on the wire it does reach.** On the loopback interface (local, or the forwarded port), the API is unauthenticated by design; anyone with local access or a foothold on the forwarded port has full dashboard access. The mitigation is the bind guard + SSH, not in-app auth.
- **Port selection is racy (TOCTOU).** `select_local_port` probes then hands the port to `ssh`; another process can claim it in between. Acceptable for a developer convenience — `ssh -L` fails loudly if so.
- **The attach-vs-spawn decision is racy too ([ADR-0353]).** Two `connect` invocations starting within the same `ATTACH_PROBE_TIMEOUT` window can both see nothing answering and both fall back to spawning; the second spawn simply fails to bind the remote port and surfaces via the existing exit-code classification. Not eliminated — accepted for a developer convenience command, same posture as the port-selection race above.
- **Every connect pays a probe round trip.** `probe_attach` runs before any spawn decision, so even the common "nothing running yet" case waits up to `ATTACH_PROBE_TIMEOUT` (5s) before falling through to the original spawn-and-wait flow.

---

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>` and later forwarded `--global` to the remote serve.
- [ORB-00030] — Workspace-keyed state, `Ws` extractor, global serve, aggregate endpoints, frontend selector.
- [ORB-00360] — Loopback-only bind guard and stored-XSS fix.
- [ORB-10029] — Made global mode the only mode for `orbit web serve`; `--global` is now a deprecated no-op kept for `connect` passthrough compatibility.
- [ORB-10200] — Moved runtime-free web dispatch into the exhaustive command-operation registry.
- [ORB-10294] — Live per-request registry refresh (§2.1): native workspace add/remove/rebind is honored without a restart, with atomic snapshot swap, runtime eviction, keep-last-valid on malformed reads, and no auto-delete. See [ADR-0234](./4_decisions.md).
- [ORB-10310] — Made every task-listing surface status-neutral and bounded by `DEFAULT_TASK_LIST_LIMIT`.
- [ORB-10319] — Moved logical-workspace catalog and registered-checkout runtime construction into `orbit-remote`; dashboard behavior is unchanged.
- [ORB-10400] — Gave `GET /api/tasks` server-side status/tag/type filters applied before the limit, plus the `{ items, total, limit, truncated }` page envelope (§2.2). Prerequisite for Bridge's `orbit_task_list` (ws_bridge ORB-10398), which could previously only filter an already-truncated array client-side.
- [ORB-10708] — Made `orbit web connect` probe for and attach to an already-running remote dashboard instead of always spawning one (§3 step 2). See [ADR-0353](./4_decisions.md).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

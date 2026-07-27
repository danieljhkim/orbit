---
title: "Remote Access — Decisions"
owner: claude
last_updated: 2026-07-18
status: Accepted
feature: remote-access
doc_role: decisions
type: design
summary: "ADR log: why live viewing supersedes a git-sync registry, why remote access is SSH-over-loopback rather than a network bind with auth, and why orbit-web reloads the registry per request rather than watching it."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**", "crates/orbit-remote/src/runtime.rs", "crates/orbit-remote/src/workspace_registry.rs"]
related_features: [remote-access, host-registry]
related_artifacts: [ADR-0200, ADR-0201, ADR-0234, ORB-00029, ORB-00030, ORB-00360, ORB-10294, ORB-10319]
---

# Remote Access — Decisions

ADR log for remote access. Entries are append-only and ordered by ascending global ID. IDs were allocated via `orbit.adr.add`; the store owns ID, status, owner, and links, and this file is the long-form narrative keyed on the same ID. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full rules.

The workspace-keyed-state machinery (the `Ws` extractor vs. path-prefixed routes) is decided in [user-interface ADR-00030](../user-interface/4_decisions.md) and not restated here.

---

## ADR-0200 — Live remote/multi-workspace viewing supersedes the git-sync task registry

**Status:** Accepted · 2026-07 · [ORB-00029], [ORB-00030]

**Context.** Orbit ships per-engineer, which leaves a coordination gap: engineer A's tasks are invisible to engineer B, and your own tasks on another machine are invisible from your laptop. The archived [task-sync](../_archive/task-sync/1_overview.md) design proposed closing it with a durable git-orphan-branch task registry plus operation-aware replay — a shared, offline-capable, *writable* store, deliberately deferred because doing it correctly is meaningful engineering. Meanwhile two smaller features shipped that answer the *viewing* half of the gap with none of that machinery: `orbit web serve --global` ([ORB-00030]) and `orbit web connect <ssh-host> [--global]` ([ORB-00029]).

**Decision.** Treat live remote/multi-workspace dashboard viewing as Orbit's answer to the cross-machine task-visibility gap, superseding the git-sync task registry. The `task-sync` folder is archived (Superseded); this `remote-access` folder documents the shipped feature. We do not build the orphan-branch registry, operation-aware replay, or registry-scoped ID allocation. The gap is addressed by viewing what already exists on each machine, not by synchronizing a shared writable store.

**Consequences.**
- One coherent, shipped story — `web serve --global` for all local workspaces, `web connect` for a remote machine, `web connect --global` for every workspace on a remote — with no server, no sync branch, and no new auth.
- The per-engineer deployment doctrine is preserved unchanged: nothing is written across machines; each machine stays the source of truth for its own tasks.
- The archived task-sync record is retained, so the rejected git-sync mechanism and the reasons it was dropped stay inspectable.
- Cost: viewing is **not** sync — it needs the target machine online and SSH-reachable, shows one machine's state at a time (the aggregate is per-machine, not cross-machine), and offers no offline, writable, or merge path. A team that genuinely needs a shared writable task registry is not served by this and would have to revisit a shared-host or sync design.

---

## ADR-0201 — Remote access is an SSH tunnel over a loopback-only bind, never a network bind with auth

**Status:** Accepted · 2026-07 · [ORB-00029], [ORB-00360]

**Context.** The dashboard exposes an unauthenticated JSON API and mutating task actions. Making it reachable from another machine has two broad shapes: (a) bind it to a routable interface and add an auth/authorization layer (tokens, sessions, reverse proxy with auth), or (b) keep it loopback-only and reach it through a transport the operator already trusts. Option (a) makes Orbit own a network-facing auth surface — credential storage, rotation, sessions — on a tool that is unauthenticated by default.

**Decision.** Remote access is option (b). The dashboard always binds loopback only ([ORB-00360]'s `check_bindable_host` refuses any non-loopback host), and `orbit web connect` reaches a remote dashboard by running `orbit web serve --no-open` on the remote over SSH and forwarding a local port through the same connection. Authentication, authorization, and transport encryption are delegated entirely to SSH. Orbit adds no token, no ACL, no session. `--global` and `--root` are forwarded to scope the tunnel, but the security posture is identical either way; the tunnel and remote serve are reaped on Ctrl-C via pty SIGHUP.

**Consequences.**
- Zero new network attack surface: the only listeners are loopback on both ends; the wire is SSH.
- The auth story is the team's existing SSH posture — nothing to provision, rotate, or leak on Orbit's side.
- Consistent with the auth stance the archived task-sync design also reached (piggyback on existing infra rather than build Orbit-specific auth).
- Cost: remote viewing requires SSH reachability and `orbit` on the remote's non-interactive PATH; there is no browser-only or tokened-URL access, and an operator who cannot SSH to a box cannot see its dashboard. SSH auth failures surface as ssh's own errors, which Orbit does not paper over.

---

## ADR-0234 — orbit-web reloads the workspace registry per request rather than watching or snapshotting once

**Status:** Accepted · 2026-07 · [ORB-10294]

**Context.** `orbit web serve` snapshotted `~/.orbit/workspaces.json` once in `build_state` and cached each workspace runtime indefinitely. A native `orbit workspace init/remove` (or a re-pointed checkout binding) succeeded on disk but stayed invisible to the web/Bridge surface until the server was restarted — a papercut that forced a restart during Build Week cleanup and contradicts the server's contract of routing the *current* registered workspace set. Three shapes were on the table: (a) keep the startup snapshot and accept restarts; (b) run a filesystem watcher daemon that reloads on `workspaces.json` change; (c) reload the registry on demand at each request boundary.

**Decision.** Option (c). A registry-backed `DashboardState` keeps a `RegistrySource` and reloads the authoritative registry via `refresh()` at each request boundary (the `Ws` extractor for all routed handlers, plus `/api/workspaces`, `/api/tasks/all`, and detailed `/healthz`). `refresh()` reloads into a fresh `Snapshot`, swaps the whole `Arc<Snapshot>` atomically under a mutex, and — serialized by a dedicated refresh lock — evicts only the runtimes whose workspace was removed, went inactive, or was rebound; runtimes are (re)built lazily in `runtime_for`, never under the registry/cache lock. A malformed or partially-written registry read retains the last valid in-memory snapshot and emits a credential-safe diagnostic (registry path + Orbit's own error, never the file body). Stale-path entries are reported inactive, never auto-deleted. The watcher daemon (b) was rejected as more moving parts (a background task, inotify/kqueue portability, debounce, its own failure modes) than a loopback dashboard reached per request needs; the request/refresh path reuses the existing choke points and carries no daemon.

**Consequences.**
- Native add/remove/rebind and post-startup path disappearance are reflected without a restart, and one broken workspace never strands the rest — eviction is per-binding and the aggregate views skip unopenable workspaces.
- Operator recovery is "fix the checkout and issue any request": a re-pointed or restored path re-activates on the next refresh, with no manual cache flush or restart. A malformed registry is self-healing — the last good set keeps serving until the file parses again.
- Startup keeps its original strictness: `from_registry`'s initial load is eager, so a malformed registry *at boot* is still fatal; only *refreshes* fall back to keep-last-valid.
- Cost: every request boundary re-reads and re-parses `workspaces.json` (cheap for a loopback dashboard, but not free), and refresh is eventually-consistent rather than strongly serialized against in-flight requests — two concurrent refreshes race to publish, and a runtime evicted by a stale race is simply rebuilt on the next request. This is acceptable precisely because the binding recheck in `runtime_for` makes a stale cache entry unservable regardless of eviction timing.

---

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>` and forwarded `--global` to the remote serve.
- [ORB-00030] — Global multi-workspace dashboard, workspace-keyed state, aggregate endpoints.
- [ORB-00360] — Loopback-only bind guard and stored-XSS fix.
- [ORB-10029] — Made global mode the default and only mode for `orbit web serve` (single mode is no longer reachable from the CLI); `--global` is now a deprecated no-op kept for `connect` passthrough compatibility. Does not change either ADR above — the security posture and viewing-not-sync boundary are unaffected — but evolves the `web serve --global` behavior both describe.
- [ORB-10294] — Live per-request registry refresh for `orbit web serve` ([ADR-0234]): native workspace add/remove/rebind without a restart.
- [ORB-10319] — Moved the dashboard's catalog/runtime dependencies into `orbit-remote`; this is an implementation-ownership change, not a new remote-access decision.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---
title: "Remote Access — Decisions"
owner: claude
last_updated: 2026-08-11
status: Accepted
feature: remote-access
doc_role: decisions
type: design
summary: "Decision log: why live viewing supersedes a git-sync registry, why remote access is SSH-over-loopback rather than a network bind with auth, and why orbit-web reloads the registry per request rather than watching it."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**", "crates/orbit-remote/src/runtime.rs", "crates/orbit-remote/src/workspace_registry.rs"]
related_features: [remote-access, host-registry]
related_artifacts: [ORB-00029, ORB-00030, ORB-00360, ORB-10294, ORB-10319, ORB-10708]
---

# Remote Access — Decisions

The workspace-keyed-state machinery (the `Ws` extractor vs. path-prefixed routes) is decided in [Global, Multi-Workspace Dashboard](../user-interface/4_decisions.md#global-multi-workspace-dashboard) and not restated here.

---

## Live remote/multi-workspace dashboard viewing supersedes the git-sync task registry

**Recorded:** 2026-07-02 05:01:07.910915Z · [ORB-00029], [ORB-00030], [ORB-00360]

Context. Orbit ships per-engineer (POSITIONING): each operator runs Orbit locally, with locks and audit DB on their own machine. That leaves a visible coordination gap — engineer A creates ORB-00042 on their laptop and engineer B has no way to see it short of asking or finding the PR. The task-sync design (now archived under docs/design/_archive/task-sync/) proposed closing that gap with a durable git-orphan-branch task registry (refs/heads/orbit/tasks) plus operation-aware replay for conflict resolution — a shared, offline-capable, WRITABLE store. It was deliberately deferred: doing it correctly requires an operation-aware-replay subsystem and a structured-conflict UX that are meaningful engineering, and a half-built version produces the wrong mental model.

Meanwhile two smaller features shipped that answer the *viewing* half of the same gap with none of that machinery: `orbit web serve --global` (ORB-00030) serves one loopback dashboard over every workspace in ~/.orbit/workspaces.json, and `orbit web connect <ssh-host> [--global]` (ORB-00029) tunnels that loopback dashboard from a remote machine over SSH. Together they let an operator see tasks across every workspace on a machine, and across machines, live.

Decision. Treat live remote/multi-workspace dashboard viewing as Orbit's answer to the cross-machine task-visibility gap, superseding the git-sync task registry design. The `task-sync` design folder is archived (status Superseded), and a new `remote-access` design folder documents the shipped viewing feature. We do NOT build the git-orphan-branch registry, operation-aware replay, or ORB-00000 allocation-against-registry described there. The gap is addressed by viewing what already exists on each machine rather than by synchronizing a shared writable store.

Consequences.
- One coherent, shipped story: `web serve --global` for all local workspaces, `web connect` for a remote machine, `web connect --global` for every workspace on a remote machine. No server, no new auth, no sync branch.
- The per-engineer deployment doctrine is preserved unchanged: nothing is written across machines; each machine remains the source of truth for its own tasks.
- The archived task-sync record is retained so the rejected git-sync mechanism (and why it was dropped) stays inspectable.
- Cost: viewing is NOT sync. It requires the target machine to be online and SSH-reachable, shows one machine's state at a time (the aggregate is per-machine, not cross-machine), and offers no offline, writable, or merge story. A team that genuinely needs a shared writable task registry is not served by this and would need to revisit a shared-host or sync design later.

## Remote dashboard access is an SSH tunnel over a loopback-only bind, never a network bind with auth

**Recorded:** 2026-07-02 05:01:07.972485Z · [ORB-00029], [ORB-00030], [ORB-00360]

Context. The dashboard exposes an unauthenticated JSON API and mutating task actions. To make it reachable from another machine there are two broad options: (a) bind it to a routable interface and add an authentication/authorization layer (tokens, sessions, reverse proxy with auth), or (b) keep the server loopback-only and reach it through an authenticated transport the operator already trusts. Option (a) means Orbit owns a network-facing auth surface — credential storage, rotation, session handling, and the blast radius of getting any of it wrong on an unauthenticated-by-default tool.

Decision. Remote access is option (b): the dashboard always binds loopback only (ORB-00360's check_bindable_host refuses any non-loopback host), and `orbit web connect <ssh-host>` reaches a remote dashboard by running `orbit web serve --no-open` on the remote over SSH and forwarding a local port through the same SSH connection. Authentication, authorization, and transport encryption are delegated entirely to SSH — whatever keys/config the operator already uses. Orbit adds no token, no ACL, no session. `--global` and `--root` are forwarded to the remote serve so the tunnel can scope to one workspace or span all of them, but the security posture is identical either way. The tunnel is torn down (and the remote serve reaped via pty SIGHUP) on Ctrl-C.

Consequences.
- Zero new network attack surface: the only listener is loopback on both ends; the wire is SSH.
- The auth story is the team's existing SSH posture — no separate credential to provision, rotate, or leak.
- Symmetry with the git-sync auth stance that the archived task-sync design also reached (piggyback on existing infra rather than build Orbit-specific auth), so the conclusion is consistent across both designs.
- Cost: remote viewing requires SSH reachability and `orbit` on the remote's non-interactive PATH; there is no browser-only or tokened-URL access path, and an operator who cannot SSH to the box cannot see its dashboard. Short-lived/again-prompting SSH auth surfaces as ssh's own errors, which Orbit does not paper over.

## orbit-web reloads the workspace registry per request rather than watching or snapshotting once

**Recorded:** 2026-07-18 10:41:51.968238Z · [ORB-10294]
**Paths:** `crates/orbit-dashboard/**`

### Context
orbit-web previously snapshotted ~/.orbit/workspaces.json once at startup and cached runtimes indefinitely, so native workspace init/remove and binding changes required a server restart. A watcher would add a resident process and synchronization surface that the loopback request path does not need.

### Decision
A registry-backed DashboardState reloads the authoritative workspace registry at each request boundary used by the Ws extractor, /api/workspaces, /api/tasks/all, and detailed /healthz. Refresh builds a complete new snapshot, swaps it atomically, and evicts only runtimes for workspaces that were removed, became inactive, or changed binding. A malformed or partial refresh retains the last valid snapshot and emits a credential-safe diagnostic; stale-path entries are reported inactive and are never auto-deleted.

### Consequences
- Native registry mutations become visible without restarting orbit-web.
- Concurrent requests observe either the previous complete snapshot or the next complete snapshot, never a partially rebuilt registry.
- Malformed refreshes remain serviceable from the last known-good snapshot and require operator correction of the registry file.
- Cost: each request boundary re-reads and parses the small registry file, and mutations are eventually consistent with requests already in flight rather than transactionally synchronized with them.

## orbit web connect attaches to an already-running remote dashboard instead of always spawning one

**Recorded:** 2026-08-10 03:04:50.454508Z · [ORB-10708]
**Paths:** `crates/orbit-dashboard/src/connect.rs`

**Context.** Every `orbit web connect` invocation unconditionally appended `orbit web serve` as the trailing remote command on its `ssh -tt` tunnel, on the assumption that nothing was already listening on `remote_port`. When a remote dashboard was already up — another engineer's still-attached tunnel, or a long-lived remote listener meant to be reused — the second invocation's remote command simply failed to bind, so a second connect either errored out or left a dead remote process behind. There was no attach path: ownership of "the tunnel" and ownership of "the remote process" were not distinguished, so nothing could safely share an already-running server.

**Decision.** Extend the existing readiness probing rather than add a parallel mechanism. `connect` first opens a bare port forward with no remote command (`ssh -N -L <local>:localhost:<remote> <host>`, via `build_probe_ssh_args`) and polls the existing `/healthz` readiness loop (`poll_until_ready`, refactored out of `wait_until_ready`) against it with a short `ATTACH_PROBE_TIMEOUT` (5s). A tunnel can come up healthy while nothing is listening behind it, so readiness is decided by the `/healthz` response, never by `ssh`'s own exit code. If something answers within the probe window, that bare forward becomes the session's `SshTunnel` outright (attach mode) and no remote command is ever sent. If nothing answers before the probe timeout, the probe forward is torn down and `connect` falls back to the original flow unchanged: a fresh `ssh -tt` session carrying `orbit web serve` as the trailing remote command, read with the existing 30s `READINESS_TIMEOUT`. Teardown classification is untouched: `SshTunnel`'s Drop-based SIGTERM/SIGKILL of the local `ssh` process is identical in both modes — in attach mode no remote command is tied to the session, so closing the forward cannot orphan or kill anyone else's process; in spawn mode the existing pty/SIGHUP path still reaps exactly the process this invocation started.

**Consequences.**
- Two engineers (or a developer session plus a long-lived remote listener) can now share one remote dashboard: the second `connect` attaches instead of failing to bind a second remote server.
- Disconnecting from an attached session never touches the pre-existing remote process, since that session never started one.
- A spawned remote process still cannot be orphaned on any exit path (error, panic, Ctrl-C) — unchanged from the prior Drop-based teardown, now exercised from either match arm in `connect`.
- Cost: every `connect` invocation now pays a probe round trip before it can spawn — up to `ATTACH_PROBE_TIMEOUT` (5s) of added latency in the common case where nothing is listening yet, before the existing spawn-and-wait flow even starts.
- Cost: the attach-vs-spawn decision is still racy (TOCTOU), consistent with the existing local-port-selection racy note in this same file — two `connect` invocations starting within the same ~5s probe window can both see nothing answering and both fall back to spawning; the second spawn then simply fails to bind the remote port, surfaced via the existing ssh-exit-code classification. Not eliminated by this change; accepted for a developer-facing convenience command.

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>` and forwarded `--global` to the remote serve.
- [ORB-00030] — Global multi-workspace dashboard, workspace-keyed state, aggregate endpoints.
- [ORB-00360] — Loopback-only bind guard and stored-XSS fix.
- [ORB-10029] — Made global mode the default and only mode for `orbit web serve` (single mode is no longer reachable from the CLI); `--global` is now a deprecated no-op kept for `connect` passthrough compatibility. Does not change either ADR above — the security posture and viewing-not-sync boundary are unaffected — but evolves the `web serve --global` behavior both describe.
- [ORB-10294] — Live per-request registry refresh for `orbit web serve` ([orbit-web reloads the workspace registry per request rather than watching or snapshotting once](#orbit-web-reloads-the-workspace-registry-per-request-rather-than-watching-or-snapshotting-once)): native workspace add/remove/rebind without a restart.
- [ORB-10319] — Moved the dashboard's catalog/runtime dependencies into `orbit-remote`; this is an implementation-ownership change, not a new remote-access decision.
- [ORB-10708] — Made `orbit web connect` probe for and attach to an already-running remote dashboard instead of always spawning one ([orbit web connect attaches to an already-running remote dashboard instead of always spawning one](#orbit-web-connect-attaches-to-an-already-running-remote-dashboard-instead-of-always-spawning-one)).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

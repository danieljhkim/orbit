---
title: "Remote Access — Decisions"
owner: claude
last_updated: 2026-07-02
status: Accepted
feature: remote-access
doc_role: decisions
type: design
summary: "ADR log: why live viewing supersedes a git-sync registry, and why remote access is SSH-over-loopback rather than a network bind with auth."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**"]
related_features: [remote-access]
related_artifacts: [ADR-0200, ADR-0201, ORB-00029, ORB-00030, ORB-00360]
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

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>` and forwarded `--global` to the remote serve.
- [ORB-00030] — Global multi-workspace dashboard, workspace-keyed state, aggregate endpoints.
- [ORB-00360] — Loopback-only bind guard and stored-XSS fix.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

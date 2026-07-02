---
title: "Remote Access — Vision"
owner: claude
last_updated: 2026-07-02
status: Accepted
feature: remote-access
doc_role: vision
type: design
summary: "Open questions for remote viewing, the prior art it draws on, and where the writable-sync boundary might be revisited."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**"]
related_features: [remote-access, user-interface]
related_artifacts: [ORB-00029, ORB-00030, ADR-0200]
---

# Remote Access — Vision

This document is forward-looking: the questions remote access leaves open, the prior art that shapes it, and the one boundary — writable cross-machine state — it deliberately does not cross. Everything here is speculation unless a task ID is attached; the shipped surface is in [2_design.md](./2_design.md).

---

## 1. Open Questions

1. **Cross-machine aggregation.** The "All workspaces" view aggregates workspaces on *one* machine. Should there be a view that fans a single browser session out across several `connect` tunnels to show many machines at once? That reintroduces multi-host coordination — the thing [ADR-0200](./4_decisions.md) chose to avoid — so the bar is high.
2. **Does the writable-sync boundary ever get revisited?** [ADR-0200](./4_decisions.md) bets that *viewing* absorbs most of the demand the archived [task-sync](../_archive/task-sync/1_overview.md) design targeted. If real teams repeatedly hit "I can see Bob's task but can't claim or edit it," the durable-registry question reopens. Uptake of `connect` is itself the demand signal to watch.
3. **Aggregate performance.** `GET /api/tasks/all` reopens every workspace store per request. At what workspace count does that need a cache or an incremental index, and what invalidates it?
4. **Persistent / multiplexed tunnels.** `connect` is one foreground tunnel torn down on Ctrl-C. Is there value in a backgrounded or auto-reconnecting tunnel, or a single tunnel multiplexing several remote workspaces on distinct ports? Only if the foreground model proves too thin in practice.
5. **Write actions over the tunnel.** The dashboard exposes some task actions. Over a `connect` tunnel those mutate the *remote* workspace. Should remote-viewed dashboards default to read-only, or is "you have SSH, so you have write" the right posture? Currently the latter, implicitly.
6. **Non-SSH transports.** Some environments front machines with an SSO proxy rather than SSH. Is a "bring your own authenticated tunnel" story (documented reverse-proxy pattern) worth blessing, or does that erode the "no new auth surface" guarantee?

---

## 2. Prior Work

### 2.1 Loopback-plus-tunnel as the safe-exposure pattern

Binding a sensitive service to loopback and reaching it over SSH port-forwarding is the well-worn answer to "unauthenticated dev service, occasional remote access": Jupyter (`jupyter notebook` + `ssh -L`), Ray/TensorBoard dashboards, and countless internal admin UIs ship exactly this guidance. Remote access automates the dance rather than inventing a mechanism — its only contribution is the readiness probe, clean teardown, and `--global` scoping.

### 2.2 Multi-tenant one-process dashboards

Serving many logical tenants (here, workspaces) from one process behind a selector is standard; the design choice that mattered was *how* requests pick their tenant. Path-prefixed routes (`/api/:workspace/...`) are the common shape; remote access instead used a query-param + extractor choke point to avoid rewriting 48 routes and every fetch — see [user-interface ADR-00030](../user-interface/4_decisions.md).

### 2.3 The rejected sibling: git-native task sync

The archived [task-sync](../_archive/task-sync/1_overview.md) design (git orphan branch + operation-aware replay, in the lineage of `git-bug` and `jj op log`) is the writable-sync path remote access chose *not* to take. It remains the reference for what a durable, offline, mergeable cross-machine task store would require, if the boundary in §1.2 is ever revisited.

---

## 3. What May Be Distinctive

Little here is novel in isolation — loopback+SSH and tenant selectors are both standard. What is arguably distinctive is the *framing decision*: treating "let me see the team's tasks" as a viewing problem solvable with existing dashboards and existing SSH, and explicitly declining to build a synchronization substrate for it. Most tools in this space reach for a server or a sync protocol; remote access reaches for a port-forward and a workspace registry, and writes down ([ADR-0200](./4_decisions.md)) exactly what that costs.

---

## 4. References

**Orbit-internal**

- [2_design.md](./2_design.md) — the shipped surfaces.
- [docs/design/user-interface/4_decisions.md](../user-interface/4_decisions.md) — ADR-00030, the workspace-keyed-state machinery decision.
- [docs/design/_archive/task-sync/](../_archive/task-sync/1_overview.md) — the superseded git-sync alternative.
- [docs/POSITIONING.md](../../POSITIONING.md) — the per-engineer doctrine that motivates a no-server design.

**External**

- OpenSSH `-L` local port forwarding — `man ssh`, the transport `connect` drives.
- Jupyter / TensorBoard "bind localhost, tunnel over SSH" guidance — the same loopback-plus-tunnel pattern in the wild.

---

## Task References

- [ORB-00029] — `orbit web connect <ssh-host>` and its `--global` passthrough.
- [ORB-00030] — Global multi-workspace dashboard and aggregate endpoints.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---
title: "Remote Access — Overview"
owner: claude
last_updated: 2026-07-18
status: Accepted
feature: remote-access
doc_role: overview
type: design
summary: "How an operator views Orbit across every local workspace and across machines, with no shared server and no new auth."
tags: [remote-access]
paths: ["crates/orbit-dashboard/**", "crates/orbit-remote/src/runtime.rs", "crates/orbit-remote/src/workspace_registry.rs", "crates/orbit-cli/src/command/web.rs", "crates/orbit-cli/src/command/operation.rs"]
related_features: [remote-access, user-interface, host-registry]
related_artifacts: [ORB-00029, ORB-00030, ORB-00360, ORB-10029, ORB-10200, ORB-10319, ADR-0200, ADR-0201]
---

# Remote Access — Overview

Remote access is how one operator sees Orbit beyond a single workspace and a single machine — every workspace registered on a box at once (`orbit web serve`, the only mode since [ORB-10029]), and a box's loopback dashboard from another machine over an SSH tunnel (`orbit web connect <ssh-host>`). It is the shipped answer to the cross-machine task-visibility gap, and it is deliberately a *viewer*, not a shared store: nothing is synchronized, merged, or written across machines. It supersedes the archived git-orphan-branch [`task-sync`](../_archive/task-sync/1_overview.md) design (see [4_decisions.md ADR-0200](./4_decisions.md)).

This document is the entry point. [2_design.md](./2_design.md) specifies the two surfaces and how they compose; [3_vision.md](./3_vision.md) names the open questions and what is deliberately unbuilt; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

Orbit ships per-engineer ([POSITIONING](../../POSITIONING.md)): each operator runs it locally, with locks and audit DB on their own machine. Two visibility gaps follow from that shape:

1. **Many workspaces, one operator.** A single machine commonly hosts several Orbit workspaces. The original dashboard bound to exactly one — you ran `orbit web serve` from inside a workspace and saw only that workspace's tasks (later reachable only via an opt-in `--global` flag; [ORB-10029] made it the only mode).
2. **Many machines, one team (or one person).** Tasks that engineer A creates on their laptop are invisible to engineer B, and your own tasks on a remote build box are invisible from your laptop.

The [archived task-sync design](../_archive/task-sync/1_overview.md) proposed closing gap 2 with a durable, writable, git-orphan-branch task registry plus operation-aware conflict replay — real engineering that was deliberately deferred. Remote access closes the *viewing* half of both gaps with none of that machinery: it serves and tunnels the dashboards that already exist, so there is no server to run, no sync branch, and no new auth. The tradeoff is explicit and load-bearing — see [§2.4](#24-viewing-is-not-sync).

---

## 2. Core Concepts

### 2.1 Global (multi-workspace) serve

`orbit web serve` serves one loopback dashboard over **every** workspace registered in `~/.orbit/workspaces.json`, regardless of the directory it is launched from; the dropdown preselects the registered workspace containing the cwd, if any, else "All workspaces". Introduced by [ORB-00030] behind an opt-in `--global` flag (single-workspace serving was the default inside a workspace); [ORB-10029] made global the only mode and `--global` a deprecated no-op, kept solely because `orbit web connect` still forwards it to the remote serve.

### 2.2 Workspace-keyed state + the `Ws` extractor

The dashboard's axum state is a workspace-keyed, lazily-built runtime map ([`DashboardState`](../../../crates/orbit-dashboard/src/state.rs)) rather than a single runtime. Each request selects its workspace through the `Ws` extractor via a `?workspace=<id>` query parameter (falling back to a configured default). Stale-path workspaces are listed but skipped, never built. The machinery decision is owned by [user-interface ADR-00030](../user-interface/4_decisions.md).

### 2.3 SSH-tunnel connect

`orbit web connect <ssh-host>` runs `orbit web serve --no-open` on the remote over a single `ssh` invocation that also forwards a local port, waits for `/healthz`, opens a browser, and tears the tunnel (and the remote serve process) down on Ctrl-C. `--root` is forwarded to the remote serve; `--global` is also forwarded but is a no-op against a post-[ORB-10029] remote (global is always on) and matters only against an older remote binary. Introduced by [ORB-00029]; the transport decision is [4_decisions.md ADR-0201](./4_decisions.md).

### 2.4 Viewing is not sync

Remote access shows what already exists on a machine that is online and reachable. It has **no** offline path, **no** write/merge across machines, and the "All workspaces" aggregate is per-machine, not cross-machine. This boundary is the core cost named in [4_decisions.md ADR-0200](./4_decisions.md); a team that needs a shared *writable* registry is not served by it.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Global (always-on) serve, state construction | [crates/orbit-dashboard/src/lib.rs](../../../crates/orbit-dashboard/src/lib.rs) | [ORB-00030], [ORB-10029] |
| Workspace-keyed runtime map + `Ws` extractor | [crates/orbit-dashboard/src/state.rs](../../../crates/orbit-dashboard/src/state.rs) | [ORB-00030] |
| Logical-workspace catalog + registered-checkout runtime construction | [crates/orbit-remote/src/workspace_registry.rs](../../../crates/orbit-remote/src/workspace_registry.rs), [crates/orbit-remote/src/runtime.rs](../../../crates/orbit-remote/src/runtime.rs) | [ORB-10319] |
| Aggregate endpoints (`/api/workspaces`, `/api/tasks/all`) | [crates/orbit-dashboard/src/api/workspaces.rs](../../../crates/orbit-dashboard/src/api/workspaces.rs) | [ORB-00030] |
| SSH tunnel, port selection, teardown | [crates/orbit-dashboard/src/connect.rs](../../../crates/orbit-dashboard/src/connect.rs) | [ORB-00029] |
| CLI runtime-free operation policy (dispatch before eager runtime init) | [crates/orbit-cli/src/command/operation.rs](../../../crates/orbit-cli/src/command/operation.rs) | [ORB-00029], [ORB-00030], [ORB-10200] |
| `web` subcommand wiring | [crates/orbit-cli/src/command/web.rs](../../../crates/orbit-cli/src/command/web.rs) | [ORB-00029] |
| Loopback-only bind guard | [crates/orbit-dashboard/src/lib.rs](../../../crates/orbit-dashboard/src/lib.rs) | [ORB-00360] |
| Header workspace selector + aggregate view | [assets/dashboard/app.js](../../../crates/orbit-dashboard/assets/dashboard/app.js) | [ORB-00030] |
| Superseded git-sync alternative | [docs/design/_archive/task-sync/](../_archive/task-sync/1_overview.md) | — |

---

## Task References

- [ORB-00029] — Added `orbit web connect <ssh-host>`: SSH-tunnel viewing of a remote machine's dashboard, later extended to forward `--global`.
- [ORB-00030] — Made the dashboard global/multi-workspace: workspace-keyed state, `Ws` extractor, serve-from-anywhere, aggregate endpoints.
- [ORB-00360] — Restricted the dashboard to loopback binds only and fixed stored XSS; the security floor remote access builds on.
- [ORB-10029] — Made global mode the default and only mode for `orbit web serve`; `--global` is now a deprecated no-op retained for `connect` passthrough.
- [ORB-10200] — Moved runtime-free web dispatch into the exhaustive command-operation registry.
- [ORB-10319] — Made the dashboard consume the Remote-owned workspace catalog and runtime factory instead of Core-owned registry composition.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

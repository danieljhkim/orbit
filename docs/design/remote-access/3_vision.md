---
title: "Remote Access — Vision"
owner: codex
last_updated: 2026-08-23
last_validated: 2026-08-15
status: Accepted
feature: remote-access
doc_role: vision
type: design
summary: "Evolution gates for remote Orbit Web access without weakening the loopback and registry boundaries."
tags: [remote-access, orbit-web, ssh]
paths: ["crates/orbit-web/**", "crates/orbit-registry/src/workspace_registry/**", "crates/orbit-cmd/src/registry_runtime.rs"]
related_features: [remote-access, user-interface, host-registry]
related_artifacts: [ORB-11008]
---

# Remote Access — Vision

Remote access should stay a thin way to reach a machine's existing Orbit Web state. It should not become an accidental synchronization service or an unauthenticated network daemon.

The federated MCP workspace surface is a proposed companion design, not a
change to the current Web or direct-SSH MCP behavior ([ORB-11008] documents
the proposal). It may aggregate live routing descriptors, but it does not make
Remote Access a replication feature.

## Evolution gates

### Network exposure

Any routable listener needs a real authentication and authorization design before the loopback guard can change. Origin checking, caller IP, SSH hostname, and possession of a URL are not substitutes.

### Long-lived connectivity

Background reconnect, tunnel multiplexing, or several remote machines in one browser may be useful only with explicit lifecycle, port ownership, failure reporting, and authority boundaries. The foreground one-tunnel model remains the baseline.

### Cross-machine state

A multi-machine aggregate of live workspace descriptors can be a routing
feature when every descriptor identifies its destination host and every
workspace-scoped call is delivered there. It must report unknown, unreachable,
unhealthy, ambiguous, and stale routes as failures; it cannot fall back to a
local or matching remote workspace. An offline view, a merged task result, or
any synchronized state remains a data-replication problem with an authoritative
store, conflict behavior, and write semantics of its own.

### Federated MCP authority

The proposed gateway may present one namespace and `orbit_workspace_list`, but
the selected destination host remains authoritative for runs, logs, scheduler
state, and mutations. For a repository with several checkouts, one declared
control-plane authority owns coordination and additional hosts are execution
bindings. The gateway must not introduce competing authorities, quorum
election, implicit failover, or silent merging of host-local state.

### Performance

Measure registry parse cost, runtime-cache churn, and aggregate endpoint latency before adding watchers or indexes. Any cache must remain subordinate to orbit-registry state and exact RegisteredRuntimeFactory bindings.

## Stable principles

- Web binds loopback unless a separate authenticated deployment boundary is designed.
- SSH local forwarding remains Web-specific; MCP direct stdio remains independent.
- The remote machine is authoritative for reads and writes served through its dashboard.
- Registry snapshots select workspaces; orbit-cmd constructs registered runtimes; Core executes domain behavior.
- Failed or stale workspace bindings are visible and isolated rather than silently redirected.

## Task References

- [ORB-11008] proposes federated MCP routing while preserving destination-host authority.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

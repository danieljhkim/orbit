---
summary: "MCP Session Context — Design"
type: design
title: "MCP Session Context — Design"
owner: codex
last_updated: 2026-08-09
last_validated: 2026-08-08
status: Accepted
feature: mcp-session-context
doc_role: design
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-remote/src/mcp/**", "crates/orbit-dashboard/src/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-10228", "ORB-10262", "ORB-10319", "ORB-10448", "ORB-10690", "ORB-10691", "ADR-0181", "ADR-0199", "ADR-0149", "ADR-0348", "ADR-0349"]
---

# MCP Session Context — Design

MCP session context separates the caller-supplied workspace address from provenance established at a trusted Orbit adapter, broker, or managed runtime boundary.

---

## 1. Initialize Metadata

Clients announce workspace with:

```json
{
  "_meta": {
    "orbit": {
      "workspace": "/absolute/path/to/repo"
    }
  }
}
```

`orbit-mcp` also accepts the compatibility key `_meta["orbit.workspace"]`. Empty strings are ignored. All other initialize metadata is ignored for trusted purposes, including workspace ID, caller/process identity, transport, capabilities, origin/call IDs, lease, role, agent/model identity, and task/run/activity/step correlation.

## 2. Storage And Thread-Through

`OrbitToolServer` stores a `ToolSessionContext` in an `RwLock` for the lifetime of one session. Each `tools/call` snapshots that context, generates exactly one unique `mcp_call_id` before name/exposure preflight, and passes the same snapshot through registry-backed dispatch.

That state is per session, not per process. A stdio server serves exactly one client for its lifetime, so the two coincide there; a listener does not. `McpSessionFactory::build_session` therefore constructs one `OrbitToolServer` per session. `McpTcpServer` hands each accepted connection its own, while the stateful Streamable HTTP session manager invokes the same factory for each initialized HTTP session. Sharing one server across sessions would let the last client to `initialize` overwrite every other client's workspace selector and return another workspace's data as a success. [ADR-0348], [ORB-10690], [ADR-0349], [ORB-10691]

The Remote-owned `BrokerMcpHost` resolves and validates the logical workspace plus any exact local checkout before constructing or selecting an `OrbitRuntime`, then forwards the trusted context into `OrbitRuntime::execute_tool_command_dispatch_with_session_context`, which places it on `ToolContext` and audit. Unknown/unexposed denial and runtime success/failure retain the same per-call context. `orbit-cli` only delegates `mcp serve` into this composition. Graph commands have no MCP or CLI surface as of ORB-10357.

The trusted fields are `workspace_id`, caller/process `machine_id` and display `host_id`, `transport`, the complete sorted `effective_capabilities` set, `origin_session_id`, `mcp_call_id`, and optional typed `leased_run {run_id, lease_id}`. A standalone stdio session is always `transport=local`, has exactly `{agent}`, and audits as `role=unverified`. Ambient `ORBIT_*` identity/correlation is ignored unless `ORBIT_MANAGED_RUN_CONTEXT` authenticates the existing managed envelope.

## 3. Workspace Resolution

`crates/orbit-tools/src/builtin/orbit/mod.rs` retains the shared builtin argument resolver:

1. If the tool input has a non-empty `workspace`, use it.
2. Else if `ToolContext.session_context.workspace` is non-empty, insert that value into the input passed to the runtime host.
3. Else return a clear `missing workspace` error.

When explicit input and session context differ, Orbit logs an info-level event and honors the explicit input. This preserves an operator escape hatch while making the mismatch visible in traces.

Before that tool-level fallback, `crates/orbit-remote/src/mcp/host.rs` resolves the same
selector to a logical workspace and, when placement requires it, an exact checkout. That
Remote preflight owns routing and authorization; it does not replace the builtin's
explicit-over-session input contract. [ORB-10262], [ORB-10319]

### 3a. The selector is advertised, not implied

Step 1 is the only step a general-purpose MCP client can reach: no shipping client lets a
caller inject `initialize.params._meta`, so a managed executor speaking through one has the
`workspace` argument and nothing else. `crates/orbit-mcp/src/adapter/schema.rs` therefore
injects an optional `workspace` string property into the advertised input schema of every
`McpToolScope::WorkspaceRequired` definition, and
`OrbitToolServer::input_schema_for` applies it to host-resolved and extension-owned schemas
alike. A tool that declares its own `workspace` parameter — `orbit.task.add` ([ADR-0149]),
`orbit.crew.list` — keeps its own description. Global-scoped tools get nothing.

Advertising at the adapter rather than in each tool's `ToolSchema` keeps the requirement
stated once, next to the scope that creates it: the broker rejects a scoped call without a
selector, so the broker's schema layer is what owns telling callers about it. [ORB-10448]

### 3b. Coordination reads follow checkout identity

A hub-placement call resolves against the coordination task registry, which partitions by the
workspace identity written to `.orbit/config.yaml` — not by the logical ID in the host
registry. `orbit workspace init` writes both from one value, so they normally coincide; for
workspaces registered before that convergence they differ (L-0098), and a validated
`ExactCheckoutBinding` carries the identity key precisely because of it.

`BrokerMcpHost::coordination_workspace_id` resolves the partition key from that binding, then
from the registered checkout's identity document, then falls back to the logical ID for a
genuinely checkoutless workspace. Friction partitioning and audit identity stay on the logical
ID. Without this, every hub-placement task tool addressed an empty partition on a diverged
workspace and reported `task not found` for tasks the checkout-local CLI served fine
(F2026-07-099). [ORB-10448]

## 4. Task Add

`orbit.task.add` advertises `workspace` as optional over the tool schema while still accepting explicit callers unchanged. The host action still receives a concrete `workspace` field because the tool wrapper resolves or rejects before dispatch.

This means existing explicit-workspace clients continue to work, while MCP clients with session context can call `orbit.task.add` without a `workspace` field.

## 5. Audit compatibility

Legacy audit `host` remains the executing process hostname and `session_id` retains its old meaning. Caller/process machine and display-host fields are additive. `origin_session_id` does not replace `session_id`. `job_run_id` remains the only run column: a trusted lease run populates it when empty or must match it; `lease_id` is additive. Migration v7 adds nullable columns and capability-set JSON without rewriting v1-v6 rows.

The trusted context carries the entire effective capability set. [ORB-10262] tests membership directly for both `tools/list` and `tools/call`; an empty set is denial and no arbitrary member, ordinal, maximum, or scalar ceiling represents authority.

## 6. Concerns & Honest Limitations

The session context covers stdio, TCP, and stateful Streamable HTTP sessions. [ORB-10690] established per-session isolation for the multi-client case; [ORB-10691] routes the HTTP session manager through that same construction seam and mounts `/mcp` on the dashboard listener. The MCP route is outside the dashboard `/api` origin middleware because non-browser MCP clients may legitimately present a non-localhost `Origin`. Loopback-only binding and rmcp's loopback `Host` validation remain in force. The endpoint carries no authentication of its own — reverse-proxy reachability and authentication are the deployment's concern — and its single effective capability is chosen at process start, defaulting to `agent` and never accepted from client metadata. [ADR-0349]

Streamable HTTP emits SSE frames incrementally at the origin. An intermediary can still buffer a correct stream, so deployment-path streaming requires separate verification. Graceful shutdown cancels MCP sessions before Axum drains HTTP connections; without that ordering an open SSE session could keep the process alive indefinitely.

The external channel carries a workspace address, not a trusted workspace ID. The local broker validates an absolute path against Git common-directory identity, `.orbit/config.yaml`, the logical registry, local role, and owner before separately populating `workspace_id`. Process cwd and `ORBIT_ROOT` are not fallbacks. Hub-link negotiation remains a later MCP Bridge unit.

## Task References

- [ORB-00256] implemented the initial session context channel and workspace resolver.
- [ORB-10228] implemented trusted provenance, anti-spoofing, capability-set propagation and audit, call correlation, and audit migration v7.
- [ORB-10262] implemented exact-checkout workspace resolution, placement preflight, capability enforcement, and runtime caching by exact binding.
- [ORB-10319] moved broker/session resolution and MCP composition into the vertical `orbit-remote` feature crate while leaving runtime audit/dispatch in Core.
- [ORB-10448] advertised the workspace selector on every workspace-scoped tool and routed hub-placement coordination reads by checkout identity, making the [ADR-0181] "clients that cannot send initialize metadata pass `workspace` explicitly" path reachable from a managed worktree activity.
- [ORB-10690] added the TCP transport and moved session construction behind `McpSessionFactory` so concurrent clients cannot observe or overwrite each other's session context ([ADR-0348]).
- [ORB-10691] mounted stateful Streamable HTTP at the dashboard `/mcp` route, isolated its middleware, defaulted it to the agent capability, and coupled MCP session cancellation to HTTP graceful shutdown ([ADR-0349]).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

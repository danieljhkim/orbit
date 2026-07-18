---
summary: "MCP Session Context — Design"
type: design
title: "MCP Session Context — Design"
owner: codex
last_updated: 2026-07-18
status: Accepted
feature: mcp-session-context
doc_role: design
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-10228", "ADR-0181", "ADR-0149"]
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

`OrbitToolServer` stores a `ToolSessionContext` in an `RwLock` for the lifetime of the stdio session. Each `tools/call` snapshots that context, generates exactly one unique `mcp_call_id` before name/exposure preflight, and passes the same snapshot through registry-backed or graph dispatch.

The CLI `RuntimeMcpHost` forwards the context into `OrbitRuntime::execute_tool_command_dispatch_with_session_context`, which places it on `ToolContext` and audit. Unknown/unexposed denial, runtime success/failure, and graph success/failure retain the same per-call context.

The trusted fields are `workspace_id`, caller/process `machine_id` and display `host_id`, `transport`, the complete sorted `effective_capabilities` set, `origin_session_id`, `mcp_call_id`, and optional typed `leased_run {run_id, lease_id}`. A standalone stdio session is always `transport=local`, has exactly `{agent}`, and audits as `role=unverified`. Ambient `ORBIT_*` identity/correlation is ignored unless `ORBIT_MANAGED_RUN_CONTEXT` authenticates the existing managed envelope.

## 3. Workspace Resolution

`crates/orbit-tools/src/builtin/orbit/mod.rs` owns the shared resolver:

1. If the tool input has a non-empty `workspace`, use it.
2. Else if `ToolContext.session_context.workspace` is non-empty, insert that value into the input passed to the runtime host.
3. Else return a clear `missing workspace` error.

When explicit input and session context differ, Orbit logs an info-level event and honors the explicit input. This preserves an operator escape hatch while making the mismatch visible in traces.

## 4. Task Add

`orbit.task.add` advertises `workspace` as optional over the tool schema while still accepting explicit callers unchanged. The host action still receives a concrete `workspace` field because the tool wrapper resolves or rejects before dispatch.

This means existing explicit-workspace clients continue to work, while MCP clients with session context can call `orbit.task.add` without a `workspace` field.

## 5. Audit compatibility

Legacy audit `host` remains the executing process hostname and `session_id` retains its old meaning. Caller/process machine and display-host fields are additive. `origin_session_id` does not replace `session_id`. `job_run_id` remains the only run column: a trusted lease run populates it when empty or must match it; `lease_id` is additive. Migration v7 adds nullable columns and capability-set JSON without rewriting v1-v6 rows.

Authorization and `tools/list` filtering test membership in the entire effective capability set. No arbitrary member, ordinal, maximum, or scalar ceiling represents authority.

## 6. Concerns & Honest Limitations

The session context currently covers stdio sessions; future HTTP or multi-session transports must preserve the same per-session isolation rather than promoting the value to process-global state.

The external channel carries a workspace address, not a trusted workspace ID. The local adapter may validate that address and separately populate `workspace_id`; explicit resolution, broker routing, and hub negotiation remain later MCP Bridge units.

## Task References

- [ORB-00256] implemented the initial session context channel and workspace resolver.
- [ORB-10228] implemented trusted provenance, anti-spoofing, capability-set membership, call correlation, and audit migration v7.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

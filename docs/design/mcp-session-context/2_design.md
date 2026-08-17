---
summary: "MCP Session Context — Design"
type: design
title: "MCP Session Context — Design"
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-session-context
doc_role: design
tags: ["mcp-session-context", "mcp", "workspace", "audit"]
paths: ["crates/orbit-common/src/types/tool.rs", "crates/orbit-mcp/src/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**", "crates/orbit-store/src/sqlite/audit_event_store/**"]
related_features: ["mcp-session-context"]
related_artifacts: []
---

# MCP Session Context — Design

## 1. Field ownership

| Field | Source | Meaning |
|---|---|---|
| workspace | Tool input or initialize metadata | Untrusted address selector |
| workspace_id | Authoritative server resolution | Stable logical workspace selected for execution |
| caller_machine_id | Local identity or SSH proxy argument | Opaque audit correlation label |
| caller_host_id | Local accepting server | Local display label; unset for a remote caller |
| process_machine_id, process_host_id | Accepting server | Machine that executes the call |
| transport | Accepting server | local or ssh-mcp |
| caller_ip | Accepting SSH environment | Best-effort first field of SSH_CONNECTION |
| trace_id | MCP adapter | Fresh correlation ID for one tools/call |

Clients cannot populate trusted fields through initialize metadata or tool JSON. Initialize accepts only the workspace selector under _meta.orbit.workspace, plus the compatibility spelling _meta["orbit.workspace"].

## 2. Local and SSH sessions

A local server derives caller and process identity from its own host identity. If no machine identity exists, the machine label is host/local and the display label falls back to the OS hostname or local.

Remote mode starts one child:

    ssh -T -- <host> "orbit mcp serve --remote-caller-machine-id '<label>'"

SSH inherits stdin, stdout, and stderr. Orbit does not parse or rewrite MCP in the proxy. The hidden caller argument marks the accepting session as ssh-mcp; its value remains audit metadata. The remote server derives its own process identity and may record the source address exposed by SSH_CONNECTION.

There is no TCP listener or reusable network session in v1.

## 3. Workspace resolution

For a workspace-scoped tool, the authoritative server chooses the first non-empty selector:

1. workspace in the tool input;
2. workspace announced during initialize;
3. otherwise, a missing-workspace error.

Process cwd is not an MCP fallback. The server resolves the selector against its registry, opens the selected local runtime, writes the resolved workspace_id into context, and normalizes an explicit workspace argument to the selected checkout path before Core dispatch.

Global tools do not require a workspace selector.

orbit.task.show is the one exception to the precedence above. Task IDs are a machine-global primary key in the coordination task registry, so a call carrying only {id} resolves the owning workspace from that registry and ignores the workspace announced at initialize — the announced workspace is ambient, like cwd, and is the right default for authoring but the wrong one for addressing an ID. A workspace passed in the tool input still wins and still filters: the call binds that workspace, and a task owned elsewhere is not found there. When the registry knows the ID but its owning checkout is unreadable or inactive, the error names that workspace rather than reporting the ID as unknown.

## 4. Adapter and tool surface

OrbitToolServer holds one context for its stdio session. Initialize may replace only the workspace selector. For every tools/call, the adapter clones the session context and mints one fresh trace_id without writing it back.

tools/list comes from the authoritative host on every request. Each definition carries a ToolSchema and one McpToolScope: Global or WorkspaceRequired. Scope controls only workspace-selector injection and server dispatch; it is not authorization metadata.

## 5. Core dispatch and audit

The server passes a resolved workspace call to Core through execute_tool_command_dispatch_with_session_context. Global calls use Core's global in-process dispatch seam. Unknown or unadvertised raw names use that same global seam and produce one denied row. Every tools/call therefore crosses one Core audit boundary exactly once with its per-call context.

Audit records include resolved workspace when applicable, caller/process metadata, transport, trace ID, and caller IP when present.

Model-authored fields with names resembling audit fields do not override the supplied ToolSessionContext.

## 6. Explicitly deferred

MCP performs no lease validation, placement routing, broker negotiation, or Orbit principal authentication.

Capability authorization is no longer deferred: Core's tool chokepoint authorizes a governed operation from the session's effective capabilities alone, and the serving process decides those once at startup — `orbit mcp serve` grants agent, `orbit mcp serve --operator` grants agent and operator ([ORB-10916], [ORB-10927]). McpCapability is still not MCP exposure metadata; `tools/list` advertises the same surface to every session.

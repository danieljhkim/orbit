---
summary: "MCP Session Context — Decisions"
type: design
title: "MCP Session Context — Decisions"
owner: codex
last_updated: 2026-08-22
last_validated: 2026-08-22
status: Accepted
feature: mcp-session-context
doc_role: decisions
tags: ["mcp-session-context", "mcp", "workspace", "audit"]
paths: ["crates/orbit-common/src/types/tool.rs", "crates/orbit-mcp/src/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**"]
related_features: ["mcp-session-context"]
related_artifacts: []
---

# MCP Session Context — Decisions

These are the current implementation choices for MCP v1.

## Workspace is an address before it is context

**Context.** A client must select server-side workspace state, but client input cannot establish a trusted logical identity.

**Decision.** Accept workspace from explicit tool input first, initialize metadata second, and the server's launch binding third. Resolve it on the authoritative server, then set workspace_id. Never fall back to the server process cwd.

**Consequences.** Explicit addressing works from any client launch directory. Missing or unknown selectors fail clearly. Cost: every workspace-scoped call pays server-side resolution.

## A managed integration binds its own workspace at launch

**Context.** The MCP protocol gives a client exactly one place to announce a workspace — `_meta` on initialize — and general-purpose clients do not send it. A session that carried no binding therefore refused every workspace-scoped call, and the agent had to discover a selector Orbit already knew.

**Decision.** `orbit mcp serve --workspace <selector>` binds the session before any client connects, and the config generators write it into the argv they register, using the logical `ws_*` ID. The binding is a default, not an authority: it never overrides an explicit per-call workspace or an announced one, and it is resolved against the registry like any other selector. A server launched without it stays fail-closed.

**Alternative rejected.** Falling back to the server process cwd. That is the retired routing model: it makes the answer depend on where the client happened to launch, and a linked worktree would silently address a different partition than its registration. The launch binding is written once, by whoever registered the integration, and names a workspace rather than a directory.

**Consequences.** A managed executor calls `orbit.task.update` with no workspace argument and lands in the workspace it was launched for. `tools/list` documents which of the two situations the caller is in. Cost: a stale binding (a workspace later deregistered or renamed) surfaces as a per-call resolution error naming the selector, rather than at startup.

## The accepting server owns provenance

**Context.** Tool JSON and initialize metadata can be authored by the model.

**Decision.** The server constructs process identity and transport. caller_machine_id is an opaque audit label; caller_ip is best-effort SSH observation. Neither is an authenticated principal.

**Consequences.** Spoofed tool fields cannot replace trusted context. Cost: audit metadata can correlate calls but cannot support authorization by itself.

## Remote MCP is direct SSH stdio

**Context.** A local MCP client needs the same remote tool surface without duplicating routing policy.

**Decision.** Use one non-PTY SSH child with inherited stdio and a hidden remote caller label. The local proxy performs no workspace resolution, checkout inspection, tool filtering, or routing.

**Consequences.** MCP bytes remain unchanged and all domain decisions occur remotely. Cost: SSH setup is paid per client session, and remote shell stdout must remain protocol-clean.

## Every call gets a server-minted trace

**Context.** Session metadata alone cannot distinguish concurrent or repeated tool calls.

**Decision.** Clone the session context and mint one fresh trace_id before each tools/call dispatch. Every call, including an unknown or unadvertised raw name, enters one Core audit seam, which persists that trace with the outcome row.

**Consequences.** Successes and failures can be correlated end to end. Cost: trace creation and propagation are mandatory for every MCP call.

## V1 defers policy authorization

**Context.** Identity transport and execution plumbing are useful before an Orbit authorization model is chosen.

**Decision.** MCP definitions carry only global-versus-workspace-required scope. MCP v1 does not authorize by capability, placement, lease, IP address, SSH label, or machine label. The kernel exposes the authoritative host's complete supported surface; Core remains the future policy seam.

**Consequences.** The skeleton stays small and avoids treating audit metadata as authority. Cost: deployments rely on access to the local process or SSH account until explicit Core authorization is designed.

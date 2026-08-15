---
summary: "MCP Session Context — Vision"
type: design
title: "MCP Session Context — Vision"
owner: codex
last_updated: 2026-07-18
last_validated: 2026-08-08
status: Accepted
feature: mcp-session-context
doc_role: vision
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-remote/src/mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-10228", "ORB-10319"]
---

# MCP Session Context — Vision

Session context should stay small and deliberate: fields belong here only when they are session-scoped, trusted by the transport boundary, and safer than repeating low-signal inputs on every tool call.

---

## 1. Open Questions

1. Should future MCP transports store context by transport session id rather than by server instance?
2. Which authenticated remote principal should strengthen caller-machine provenance beyond the accepted same-user SSH posture?
3. Which additional field, if any, has a trusted adapter/runtime source and a real session lifetime rather than being domain input?

## 2. Prior Work

### Task Artifacts

Surviving references to a decision whose original body was lost to worktree reaping record that `.orbit/config.yaml` stores the load-bearing `workspace_id` binding and that defaulting task writes from cwd can silently route to the wrong workspace. This document preserves only that already-recorded claim; it does not reconstruct the missing rationale.

The same incident temporarily orphaned the bodies now preserved as [MCP ambient workspace session context](./4_decisions.md#mcp-ambient-workspace-session-context), [The v2 shell activity surface is removed, not sandboxed](../activity-job/4_decisions.md#the-v2-shell-activity-surface-is-removed-not-sandboxed), [Default Claude to opus/sonnet CLI aliases; centralize model defaults in orbit-common::model_defaults](../agent-families/4_decisions.md#default-claude-to-opussonnet-cli-aliases-centralize-model-defaults-in-orbit-commonmodeldefaults), and [PR handoff recovery follows job checkpoints and exact remote leases](../activity-job/4_decisions.md#pr-handoff-recovery-follows-job-checkpoints-and-exact-remote-leases).

### MCP Schema Trimming

[ORB-00255] motivated reducing repetitive tool fields, but workspace could not become optional until MCP had a deliberate ambient channel.

## 3. What May Be Distinctive

Orbit treats session context as a safety mechanism rather than a convenience cache. The resolver deliberately refuses to use process cwd, even when cwd would appear to work, because worktree and non-default binding cases are exactly where an implicit fallback is most dangerous.

[ORB-10228] extends that principle: model-authored initialize/tool JSON cannot become identity, capability, transport, session/call, lease, or run authority. The full capability set is explicit and non-hierarchical, while legacy audit authorities remain unchanged.

## 4. References

- The lost task-artifact decision is not reconstructed here; only its surviving workspace-binding claim is retained above.
- [MCP ambient workspace session context](./4_decisions.md#mcp-ambient-workspace-session-context) records the MCP ambient workspace session context decision.
- [ORB-00256] implemented the first session context field.
- [ORB-10228] established the trusted context and audit boundary.
- [ORB-00255] motivated the schema trimming pressure that made a safe default useful.

## Task References

- [ORB-00255] motivated reducing repetitive workspace boilerplate.
- [ORB-00256] implemented the session context channel.
- [ORB-10228] implemented trusted session provenance and correlation.
- [ORB-10319] made `orbit-remote` the owner of trusted broker/session composition over the generic MCP transport kernel.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

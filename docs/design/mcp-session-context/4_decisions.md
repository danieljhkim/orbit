---
summary: "MCP Session Context — Decisions"
type: design
title: "MCP Session Context — Decisions"
owner: codex
last_updated: 2026-05-22
status: Draft
feature: mcp-session-context
doc_role: decisions
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ADR-0181", "ADR-0149"]
---

# MCP Session Context — Decisions

ADR log for MCP session context. Format follows [docs/design/CONVENTIONS.md §4](../CONVENTIONS.md): each entry is `Context · Decision · Consequences`, every entry names at least one Cost, and numbers are append-only.

---

## ADR-0181 — MCP ambient workspace session context

**Status:** Proposed · 2026-05 · [ORB-00256]

**Context.** MCP tools need CLI-like workspace ergonomics, but [ADR-0149] makes process-cwd defaults unsafe because worktree cwd can bind to a different `workspace_id`. The viable alternatives were per-call workspace input forever, a one-shot workspace lookup tool that clients cache, or a deliberate session-level signal from the MCP client.

**Decision.** MCP clients announce the canonical workspace path in `initialize.params._meta.orbit.workspace`. `orbit-mcp` stores that value in the server session context for the stdio session and passes it through `ToolSessionContext` into `ToolContext`; workspace-taking tools resolve explicit input first, then session context, then return a clear missing-workspace error. If explicit input and session context differ, the tool logs the mismatch at info level and honors explicit input.

**Consequences.**
- [ADR-0149] remains the `workspace_id` binding invariant; this ADR amends only how MCP calls address that binding.
- `orbit.task.add` and future workspace-taking tools can make `workspace` optional without defaulting to process cwd.
- Clients that cannot send initialize metadata can continue passing `workspace` explicitly.
- Cost: Orbit now carries MCP session metadata across the adapter, CLI host, runtime dispatch, and tool context, so new host surfaces must preserve that thread-through path.

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

---
summary: "MCP Session Context — Decisions"
type: design
title: "MCP Session Context — Decisions"
owner: codex
last_updated: 2026-06-21
status: Draft
feature: mcp-session-context
doc_role: decisions
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-00406", "ADR-0181", "ADR-0199", "ADR-0149"]
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

## ADR-0199 — Workspace_path-addressable MCP host tools with surface-scoped containment

**Status:** Proposed · 2026-06 · [ORB-00406]

**Context.** MCP host tools (`orbit.task.*`, `orbit.adr.*`, `orbit.learning.*`, `orbit.friction.*`, `orbit.search`) bind to a single `OrbitRuntime` resolved at `serve` launch from cwd discovery; when none is found the server installs `EmptyMcpHost` and advertises an empty `tools/list`, so clients that launch the server without the repo as cwd lose the entire host surface (e.g. Cowork, which launches with cwd / `CLAUDE_PROJECT_DIR` set to an internal scratchpad; see [ORB-00405] and learning L-0065). [ADR-0181] already routes workspace *addressing* per-call → session-context, but tool *registration* still gates on launch discovery. The real alternatives were to keep launch-gated registration and require every client to fix cwd, or to advertise host tools unconditionally and resolve the runtime per call.

**Decision.** Advertise the host tool schemas unconditionally and resolve the target `OrbitRuntime` per call via the [ADR-0181] chain (explicit `workspace_path` → session context → clear missing-workspace error), reusing `OrbitRuntime::try_initialize_existing`. Containment is scoped by surface by design: the graph adapter keeps its strict anchor containment ([2_design.md] / [ORB-00361]) because it indexes the entire worktree and is therefore a path-traversal (CWE-22) read surface; host tools, which only operate on a structured `.orbit/` store, validate only that the resolved path is an initialized Orbit workspace and honor the [ADR-0149] `workspace_id` binding — no traversal anchor is imposed on them.

**Consequences.**
- `tools/list` returns the full host surface even when launch discovery finds nothing; execution binds to the caller-supplied workspace, making Orbit usable from any MCP client regardless of launch cwd / `CLAUDE_PROJECT_DIR` and removing the per-user `--root` workaround (L-0065).
- The graph adapter's strict containment is retained and explicitly justified as tree-indexing-specific, so the asymmetry between graph and host surfaces is intentional rather than an oversight.
- Omitting `workspace_path` preserves current CLI behavior (resolve from cwd / `--root`).
- Cost: the host runtime stops being a single process-lifetime singleton — the server must resolve and cache a runtime per workspace and keep the [ADR-0181] session thread-through correct across that cache, adding per-call resolution state that every future host tool must respect.

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.
- [ORB-00406] proposes workspace_path-addressable host tools ([ADR-0199]).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

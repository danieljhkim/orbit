## Context
MCP tools need CLI-like workspace ergonomics, but ADR-0149 makes process-cwd defaults unsafe because worktree cwd can bind to a different workspace_id. The viable alternatives were per-call workspace input forever, a one-shot workspace lookup tool that clients cache, or a deliberate session-level signal from the MCP client.

## Decision
MCP clients announce the canonical workspace path in initialize.params._meta.orbit.workspace. orbit-mcp stores that value in the server session context for the stdio session and passes it through ToolSessionContext into ToolContext; workspace-taking tools resolve explicit input first, then session context, then return a clear missing-workspace error. If explicit input and session context differ, the tool logs the mismatch at info level and honors explicit input.

## Consequences
- ADR-0149 remains the workspace_id binding invariant; this ADR changes only how MCP calls address that binding.
- orbit.task.add and future workspace-taking tools can make workspace optional without defaulting to process cwd.
- Clients that cannot send initialize metadata can continue passing workspace explicitly.
- Cost: Orbit now carries MCP session metadata across the adapter, CLI host, runtime dispatch, and tool context, so new host surfaces must preserve that thread-through path.
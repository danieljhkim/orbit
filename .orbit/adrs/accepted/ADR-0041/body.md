## Context
CLI `AuditGuard` historically wrote tool-invocation audit rows, leaving MCP `tools/call` dispatch and MCP preflight failures outside the SQLite command-audit trail.

## Decision
Move tool-invocation audit to `OrbitRuntime::execute_tool_command_dispatch`, tag dispatches as CLI `"run"` or MCP `"run-mcp"`, bracket MCP preflight failures in `audited_mcp_call`, and use a per-thread signal so CLI guard rows are not duplicated.

## Consequences
- CLI and MCP tool calls, including unknown/unexposed MCP failures, now produce one audit row with shared identity resolution.
- Cost: the dedup signal is thread-local; future async or cross-thread guarded entry points must re-evaluate the boundary.

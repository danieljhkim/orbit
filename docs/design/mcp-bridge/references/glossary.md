---
type: glossary
summary: Glossary — Orbit MCP Bridge
last_updated: 2026-08-15
last_validated: 2026-08-15
---

# Glossary — Orbit MCP Bridge

| Term | Meaning |
|---|---|
| Accepting machine | The machine running `orbit mcp serve`; it resolves registry state, opens the runtime, and executes the request. |
| Caller machine ID | An opaque audit label forwarded by the local proxy. It is not an authenticated identity or authorization principal. |
| Caller IP | Best-effort audit metadata derived from `SSH_CONNECTION` on an SSH-started server. |
| Direct SSH stdio | One non-PTY SSH child carrying MCP bytes through inherited stdin and stdout without a TCP listener or frame relay. |
| Process identity | The persisted machine and display-host identity of the accepting server process. |
| Remote proxy | The small `orbit-mcp` adapter that starts direct SSH stdio and waits for its exit. |
| Trace ID | A fresh identifier created for one MCP tool call and persisted with its audit outcome. |
| Workspace selector | Caller-supplied addressing input resolved against the accepting machine's registry before execution. |

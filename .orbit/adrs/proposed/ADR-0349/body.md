## Context
Orbit needs a network MCP endpoint with incremental streaming and graceful session shutdown. The existing dashboard is the only long-running HTTP process and already owns loopback binding, Axum routing, and shutdown; alternatives were a second daemon or a separate raw-TCP port in the dashboard process.

## Decision
Mount a stateful Streamable HTTP MCP service at `/mcp` on the dashboard Axum listener. Keep it outside the `/api` router browser-origin middleware, retain loopback-only binding and MCP Host validation, select one trusted capability at process start, and cancel all MCP sessions before Axum drains HTTP connections.

## Consequences
- MCP and dashboard traffic share one listener and lifecycle while retaining separate request middleware.
- Reverse proxies remain responsible for authentication and may buffer otherwise-correct origin streaming; deployment-path streaming needs separate verification.
- Cost: the dashboard process now owns MCP availability, so dashboard restarts interrupt MCP sessions and shutdown wiring must coordinate both transports.
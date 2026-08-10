**Context**

Orbit's MCP protocol handling is already transport-agnostic: the `ServerHandler` in `orbit-mcp` performs no IO, and framing, capability filtering, and the trusted session envelope are computed entirely from the injected host and composition. The crate nonetheless bound stdio only, so an MCP client that is not a locally-spawned child process could not reach the surface at all. Remote consumers therefore entered through the dashboard HTTP API, which sits below both MCP capability filtering and the governed-operation role check. Operator-classified tools were simultaneously unreachable by their intended caller and unenforced on the path actually in use.

The blocker was not protocol work but ownership of session state. `OrbitToolServer` holds `session_context` in an `RwLock` and `initialize` writes the client's announced workspace selector into it. One stdio process serves exactly one client for its lifetime, so a single instance is correct there. A listener serving concurrent clients on one shared instance would let the last client to initialize redirect every other client's tool calls — and the resulting response is a *success* carrying another workspace's data, not an error. That cannot be detected from inside a request; it has to be impossible by construction.

**Decision**

Add a TCP listener transport to `orbit-mcp` alongside stdio, seamed at session construction rather than inside the protocol or dispatch layers.

1. `McpSessionFactory` captures the host, a trusted `ToolSessionContext` template, and the (cloneable) `McpServerComposition`, and builds one `OrbitToolServer` per session. `McpTcpServer::bind` / `serve` accepts connections and hands each its own server on its own task. Nothing mutable is shared between sessions: workspace selector, name map, and correlation ids are per connection.
2. Per-session construction clears `origin_session_id` and `mcp_call_id` from the template so the adapter mints a fresh origin id per session. A listener-wide correlation id would collapse concurrent clients into one audit identity.
3. The capability set is whatever the caller that starts the endpoint placed in the trusted context. There is deliberately no capability-free convenience entry point for the network transport, and nothing a client sends can widen it: `initialize` continues to overwrite only the legacy `workspace` selector, exactly as on stdio.
4. Session context stays `trusted_local`. No transport discriminant variant is added and no client-asserted caller identity is accepted; the endpoint serves a trusted-local session to whoever reaches the socket.
5. Authentication and reachability are the deployment's concern — loopback bind, firewalling, or an authenticating proxy — not this crate's.

**Rejected alternatives**

- *Share one `OrbitToolServer` across connections and key session state by connection id.* This keeps a single mutable map at the exact seam that must not be shared, re-introducing the failure mode as a lookup bug instead of a race, and it forces a connection identifier through the protocol layer this change deliberately left alone.
- *Add an `McpTransport::Network` discriminant and derive policy from it.* Existing transport checks compare the discriminant by equality rather than exhaustively, so a new variant compiles cleanly while silently falling out of every check written against the current set. Hub/spoke remote-session semantics also belong to `orbit-remote`, not to the transport kernel.
- *Reuse the dashboard HTTP API as the remote MCP path.* That is the status quo being corrected: it bypasses capability filtering and the governed-operation role check.
- *Ship rmcp's streamable-http/SSE server transport.* It requires an HTTP server dependency in a crate whose entire dependency surface is `orbit-common` plus `rmcp`, and it prices in a session-management model before the isolation question above was settled. A raw socket answers the reachability problem with the framing the crate already speaks. HTTP transports stay open as follow-up once authentication is in scope.
- *Default the endpoint's capability to the most privileged value for convenience.* An endpoint reachable over a socket that defaults to operator inverts the safe default; the caller that starts the server chooses.

**Consequences**

- Remote MCP consumers can reach the same filtered surface local ones do, so operator-classified tools become both reachable by their intended caller and enforced on the path in use.
- Session isolation is structural: it is impossible to serve two clients from one server instance through this API, because `build_session` returns an owned server and `serve` consumes it. A crate integration test asserts the failure mode directly (verified failing against a deliberately shared session context, where the first client silently received the second client's workspace).
- The protocol and dispatch layers are untouched apart from widening two session-context accessors to `pub(crate)` for the isolation tests.
- **Cost:** the endpoint is unauthenticated by construction, so a deployment that binds it to a non-loopback address without fronting it hands trusted-local session context — including whatever capability it was started with — to anyone who can open a socket. This crate cannot detect that misconfiguration, and the safe default (loopback) is documentation, not enforcement.
- **Cost:** per-session construction re-clones the composition (its `Vec`s of `Arc` registrations) on every accept, so a connection-churn workload pays allocation the stdio path never did; the accept loop is also unbounded, with no connection cap or backpressure.
- **Cost:** an `origin_session_id` supplied by a caller is now silently discarded on the network path. Any future consumer that expects to pin a listener-wide session id will find its value replaced per connection.
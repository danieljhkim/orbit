---
summary: "MCP Session Context — Decisions"
type: design
title: "MCP Session Context — Decisions"
owner: codex
last_updated: 2026-08-14
last_validated: 2026-08-08
status: Accepted
feature: mcp-session-context
doc_role: decisions
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-remote/src/mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool/**", "crates/orbit-cli/src/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-00406", "ORB-10228", "ORB-10262", "ORB-10319", "ORB-10448", "ORB-10690", "ORB-10758", "ORB-10769", "ADR-0181", "ADR-0199", "ADR-0149", "ADR-0348", "ADR-0361"]
---

# MCP Session Context — Decisions

> **Retired learning clauses:** [ORB-10736] / [ADR-0359] removed the native
> project-learning resource. Learning-specific examples in earlier entries are
> historical context only and are not part of the current MCP surface.

ADR log for MCP session context. Format follows [docs/design/CONVENTIONS.md §4](../CONVENTIONS.md): each entry is `Context · Decision · Consequences`, every entry names at least one Cost, and numbers are append-only.

Historical note ([ORB-10479]): the entries listed below already held a global ADR allocation, but their store bodies were lost when the worktrees that authored them were reaped (see [F2026-07-163]). The narratives were restored into the store at their existing IDs — no ID was reallocated — and their headings reduced to pointer form. Restored here: [ADR-0181], [ADR-0199].

---

## ADR-0181 — MCP ambient workspace session context

**Status:** Accepted · 2026-08-01 19:14:48.339444Z · [ORB-00256], [ORB-10228], [ORB-10448], [ORB-10479]
**Owner:** codex
**Created:** 2026-08-01 19:14:46.845232Z
**Last updated:** 2026-08-01 19:17:30.377870+00:00
**Related features:** `mcp-session-context`
**Tags:** `mcp-session-context`

**Context.** MCP tools need CLI-like workspace ergonomics, but [ADR-0149] makes process-cwd defaults unsafe because worktree cwd can bind to a different `workspace_id`. The viable alternatives were per-call workspace input forever, a one-shot workspace lookup tool that clients cache, or a deliberate session-level signal from the MCP client.

**Decision.** MCP clients announce the canonical workspace path in `initialize.params._meta.orbit.workspace`. `orbit-mcp` stores that value in the server session context for the stdio session and passes it through `ToolSessionContext` into `ToolContext`; workspace-taking tools resolve explicit input first, then session context, then return a clear missing-workspace error. If explicit input and session context differ, the tool logs the mismatch at info level and honors explicit input.

**Trusted-provenance amendment.** The announced workspace is only the legacy untrusted address selector. An Orbit adapter/runtime separately injects validated `workspace_id`, caller/process machine and display-host identity, transport, the full canonical effective capability set, origin session, exactly one call ID per call, and optional typed leased-run correlation. External metadata/tool JSON cannot populate those fields or audit identity/correlation. Standalone stdio is local, exactly `{agent}`, and `unverified`; authenticated managed-envelope fields win when the existing managed marker is present.

**Consequences.**
- [ADR-0149] remains the `workspace_id` binding invariant; this ADR amends only how MCP calls address that binding.
- `orbit.task.add` and future workspace-taking tools can make `workspace` optional without defaulting to process cwd.
- Clients that cannot send initialize metadata can continue passing `workspace` explicitly.
- Cost: Orbit now carries MCP session metadata across the generic adapter, Remote host, runtime dispatch, and tool context, so new host surfaces must preserve that thread-through path.
- Capability authorization is membership in the complete set; scalar or max-capability representations are forbidden.
- Existing audit `host`, `session_id`, and `job_run_id` meanings remain canonical; all new provenance is additive.

## ADR-0199 — Workspace_path-addressable MCP host tools with surface-scoped containment

**Status:** Accepted · 2026-08-01 19:14:51.087154Z · [ORB-00406], [ORB-10262], [ORB-10448], [ORB-10479]
**Owner:** codex
**Created:** 2026-08-01 19:14:49.856034Z
**Last updated:** 2026-08-01 19:17:33.481247Z
**Related features:** `mcp-session-context`
**Tags:** `mcp-session-context`

**Context.** MCP host tools (`orbit.task.*`, `orbit.adr.*`, `orbit.learning.*`, `orbit.friction.*`, `orbit.search`) bind to a single `OrbitRuntime` resolved at `serve` launch from cwd discovery; when none is found the server installs `EmptyMcpHost` and advertises an empty `tools/list`, so clients that launch the server without the repo as cwd lose the entire host surface (e.g. Cowork, which launches with cwd / `CLAUDE_PROJECT_DIR` set to an internal scratchpad; see [ORB-00405] and learning L-0065). [ADR-0181] already routes workspace *addressing* per-call → session-context, but tool *registration* still gates on launch discovery. The real alternatives were to keep launch-gated registration and require every client to fix cwd, or to advertise host tools unconditionally and resolve the runtime per call.

**Decision.** Advertise canonical schema-plus-policy definitions unconditionally and resolve the target route per call via the [ADR-0181] chain (non-empty explicit `workspace` → session context → clear missing-workspace error). Absolute local paths are validated against the logical registry, Git common directory, and `.orbit/config.yaml`; relative paths, process cwd, and `ORBIT_ROOT` are never routing inputs. Exact checkout identity is retained for graph containment and runtime caching.

**Consequences.**
- `tools/list` returns the full host surface even when launch discovery finds nothing; execution binds to the caller-supplied workspace, making Orbit usable from any MCP client regardless of launch cwd / `CLAUDE_PROJECT_DIR` and removing the per-user `--root` workaround (L-0065).
- The graph adapter's strict containment is retained and explicitly justified as tree-indexing-specific, so the asymmetry between graph and host surfaces is intentional rather than an oversight.
- Omitting workspace selection is an actionable error; MCP never inherits CLI cwd/`--root` discovery.
- Cost: the host runtime stops being a single process-lifetime singleton — the server must resolve and cache a runtime per workspace and keep the [ADR-0181] session thread-through correct across that cache, adding per-call resolution state that every future host tool must respect.

## ADR-0348 — Serve MCP over TCP with one server instance per session

**Status:** Proposed · 2026-08-09 22:36:28.072916Z · [ORB-10690]
**Owner:** claude
**Created:** 2026-08-09 22:36:28.072916Z
**Last updated:** 2026-08-09 22:36:28.072916Z
**Related features:** `mcp-session-context`
**Tags:** `mcp`, `transport`, `capability`
**Paths:** `crates/orbit-mcp/**`

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

## ADR-0349 — Host Streamable HTTP MCP on the dashboard listener

**Status:** Proposed · 2026-08-09 22:58:44.842499Z · [ORB-10691]
**Owner:** codex
**Created:** 2026-08-09 22:58:44.842499Z
**Last updated:** 2026-08-09 22:58:44.842499Z
**Related features:** `mcp-session-context`
**Tags:** `mcp`, `transport`, `dashboard`
**Paths:** `crates/orbit-dashboard/src/**/*.rs`, `crates/orbit-remote/src/mcp/**/*.rs`, `crates/orbit-mcp/src/**/*.rs`

### Context
Orbit needs a network MCP endpoint with incremental streaming and graceful session shutdown. The existing dashboard is the only long-running HTTP process and already owns loopback binding, Axum routing, and shutdown; alternatives were a second daemon or a separate raw-TCP port in the dashboard process.

### Decision
Mount a stateful Streamable HTTP MCP service at `/mcp` on the dashboard Axum listener. Keep it outside the `/api` router browser-origin middleware, retain loopback-only binding and MCP Host validation, select one trusted capability at process start, and cancel all MCP sessions before Axum drains HTTP connections.

### Consequences
- MCP and dashboard traffic share one listener and lifecycle while retaining separate request middleware.
- Reverse proxies remain responsible for authentication and may buffer otherwise-correct origin streaming; deployment-path streaming needs separate verification.
- Cost: the dashboard process now owns MCP availability, so dashboard restarts interrupt MCP sessions and shutdown wiring must coordinate both transports.

## ADR-0361 — One workspace selector grammar on CLI and MCP

**Status:** Accepted · 2026-08 · [ORB-10758]
**Code anchors:** `crates/orbit-remote/src/workspace_registry.rs::resolve_logical_workspace`, `crates/orbit-remote/src/runtime.rs::initialize_with_overrides`, `crates/orbit-remote/src/mcp/host.rs::resolve_workspace`, `crates/orbit-cli/src/command/mod.rs::Cli`

### Context

MCP `resolve_workspace` already accepted a logical workspace ID or an absolute checkout path and never inherited process cwd ([ADR-0149], [ADR-0181], [ADR-0199]). The CLI had no equivalent: `orbit --help` exposed only `--root` (a data-directory override), and `orbit.task.add --workspace` rejected `ws_*` ids while the MCP schema advertised them. Two grammars for one concept meant guidance that was correct on one surface was wrong on the other.

### Decision

Every workspace-routing surface accepts the same three selector forms: a registered workspace name (`orbit`), a logical ID (`ws_orbit`), or an absolute checkout path (a linked Git worktree resolves to its registered checkout). Ambiguous or unknown selectors fail closed and name the rejected value.

Resolution order is unchanged: explicit per-call / `--workspace` selector, then MCP initialize `_meta.orbit.workspace`, then CLI cwd discovery. MCP never falls back to process cwd. `--root` stays a data-directory override; the new top-level `orbit --workspace <selector>` is the CLI selector and is not clap-global so it does not collide with `task add --workspace` (a repository-relative task path). A routed audit event records the resolved `workspace_id`, not only the raw selector.

### Rejected alternatives

- *MCP cwd fallback.* Already rejected by [ADR-0149] / [ADR-0181]: a worktree cwd can bind a different `workspace_id` than the caller named.
- *Overloading `--root` as a workspace selector.* `--root` is the data-directory escape hatch and already pins the global registry root; making it also mean "this checkout" would collapse two independent overrides.
- *A stateful `switch_workspace` MCP tool.* Mutating session context mid-connection is how TCP sessions already go wrong ([ADR-0348]); per-call `workspace` and CLI `--workspace` are the dynamic path.

### Consequences

- Agents can name a workspace the same way on `orbit --workspace` and on every workspace-scoped MCP tool.
- `orbit.task.add`'s advertised `workspace` parameter matches the grammar the broker actually accepts.
- Cost: name collisions across registered workspaces become hard errors instead of first-match; operators with duplicate names must disambiguate by id or path.

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.
- [ORB-00406] proposes workspace_path-addressable host tools ([ADR-0199]).
- [ORB-10690] added the MCP TCP transport with per-session server construction ([ADR-0348]).
- [ORB-10228] accepted and implemented the trusted-provenance amendment to [ADR-0181].
- [ORB-10262] accepted and implemented ADR-0199 through the exact-checkout local broker.
- [ORB-10319] consolidated the broker/session implementation in `orbit-remote`; it does not change ADR-0181 or ADR-0199 semantics.
- [ORB-10448] made both ADRs reachable from a managed worktree activity: the `workspace` selector is now advertised on every workspace-scoped tool, and hub-placement coordination reads address the checkout-identity partition. Neither changes ADR-0181 or ADR-0199 semantics; see [2_design.md §3a–3b](./2_design.md). The advertised-selector contract is a breaking `tools/list` schema change (RELEASING.md) and may warrant its own allocated ADR — this task's activity was not granted `orbit.adr.add`, so no global ID was allocated.
- [ORB-10758] unified the workspace selector grammar across the CLI `--workspace` flag and MCP workspace-scoped tools ([ADR-0361]).
- [ORB-10769] bound CLI `orbit tool run` to the same fail-closed workspace selector above the tools.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

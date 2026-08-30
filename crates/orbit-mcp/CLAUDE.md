# orbit-mcp

Project instructions for the Model Context Protocol crate.

## One job

Speak MCP. This crate owns stdio framing, advertised-name translation,
structured responses, per-call trace creation, canonical tool discovery, server
identity presentation, the TCP listener, the direct SSH stdio proxy, and the
federated mux. It is a protocol crate, not a runtime.

Everything a protocol should not decide stays behind the [`McpHost`](src/lib.rs)
trait: workspace resolution, domain validation, auditing, and authorization.
The kernel hands the host a canonicalized call plus one trusted
`ToolSessionContext` and returns what it gets back. If you find yourself
writing a rule about *whether* a call is allowed, it belongs in `orbit-core`.

## Dependency boundary is a test

[`tests/dep_boundary.rs`](tests/dep_boundary.rs) asserts the exact set of
internal dependencies (`orbit-common`, `orbit-registry`, `orbit-tools`,
`orbit-types`) by parsing this crate's manifest. Adding an edge to a command,
runtime, or Web crate fails that test on purpose — the fix is a host method,
not a wider manifest.

`rmcp` appears in this crate and nowhere else in the workspace. Keep it that
way: translate `rmcp` types at the adapter edge rather than re-exporting them,
so a protocol-library upgrade stays a change in one crate.

## Internal layout

- [`adapter/`](src/adapter) — the MCP server itself:
  [`dispatch`](src/adapter/dispatch.rs) routes a call,
  [`name_map`](src/adapter/name_map.rs) translates canonical Orbit tool names to
  the advertised character set and detects collisions,
  [`schema`](src/adapter/schema.rs) composes JSON Schema, and
  [`structured`](src/adapter/structured.rs) shapes responses.
- [`remote/`](src/remote) — server identity, canonical discovery, and the
  byte-transparent SSH proxy for a caller that has already chosen one host.
- [`federated/`](src/federated) — the mux: `config` (operator destinations),
  `descriptor` (live workspace descriptors), `probe` (in-process and SSH
  destinations), `capability`, and `host` (`FederatedMcpHost`).
- [`listener.rs`](src/listener.rs) — the TCP transport and its exposure gate.
- [`error.rs`](src/error.rs) — error shaping toward MCP clients.

Tests use sibling `tests/` directories
([`test_layout.md`](../../docs/design-patterns/test_layout.md)); crate-root
[`tests/`](tests) holds the dependency-boundary assertion and the wire
round-trip that drives a real client over an in-memory duplex.

## Crate-specific invariants

- **Advertised names are shipped contract.** Sanitization exists because Cursor
  accepts `[a-zA-Z0-9_]` and VS Code accepts `[a-z0-9_-]`; the mapping keeps
  Orbit's names inside that intersection *without renaming any canonical
  identifier*. A collision after sanitization is a hard error, never a silent
  rename.
- **The federated mux is the one place Orbit is also an MCP client.** Remote
  membership comes only from the operator's destinations file; the accepting
  machine is an implicit local destination, never an SSH row. It is not a fleet
  registry and answers are not cached between calls.
- **Routing is fail-closed.** A federated call carries a host-qualified
  selector the caller copied from federated `orbit.workspace.list`. A local
  selector is delivered in-process without SSH; anything else is refused as
  `unknown_selector` rather than guessed at.
- **The accepting machine resolves local state.** Whatever the transport —
  stdio, TCP, SSH proxy, or a routed federated call — the machine that accepts
  the call resolves its own workspaces and dispatches through Core.

## Context

Orbit's MCP protocol handling is already transport-agnostic — the server handler performs no IO, and stdio is bound in a single function. But stdio is the only binding, so a client that is not a local child process cannot reach the MCP surface at all.

Remote consumers therefore enter through the dashboard HTTP API instead. That API invokes runtime services directly, below both enforcement layers: MCP capability filtering and the governed-operation role check. Two consequences follow.

First, operator-classified operations are simultaneously unreachable by their intended caller and unenforced on the path actually in use. The capability model is expressible but not load-bearing.

Second, every remote consumer must re-implement orbit's response semantics — structured-content shaping, record projection, workspace resolution, error-text mapping, strict argument handling — in its own language against a hand-maintained schema fixture. That reimplementation has required a follow-up change for roughly every orbit MCP schema change, and has twice diverged behaviourally in ways a schema diff could not detect. The existing initialize-time contract digest already solves this class of problem for native clients, by hard-failing mismatched builds.

## Decision

Add a network MCP transport alongside stdio, and host it on the existing long-running server process.

- One server instance is constructed per client session. Session state that today lives on the shared server struct and is mutated at initialize must not be shared across concurrent clients.
- The endpoint stamps trusted-local session context and selects capability at serve time. Capability does not default to the most privileged value.
- Hub/spoke remote-session semantics are out of scope: no new transport discriminant, no client-asserted caller identity.
- Network authentication is the deployment's concern, not orbit's. Orbit keeps its loopback-only bind.

Alternatives considered and rejected:

- **A standalone MCP daemon.** Adds a second long-lived unit, a second bind/TLS story, and a second place workspace-registry refresh must happen, for no isolation benefit. The existing server already refreshes registry state per request boundary and already ships graceful shutdown and streaming.
- **Extending hub/spoke over the network transport.** Hub trust derives from process provenance: the peer exists only because the caller was authenticated before the process was spawned. A shared network credential cannot bind an asserted caller identity, and at least one tool privilege is gated on the transport discriminant alone. Existing checks compare that discriminant by equality rather than exhaustively, so a new variant would compile cleanly while behaving wrong.
- **Keeping the external proxy and adding a capability gate to the HTTP API.** Builds a second enforcement layer to compensate for a second access path, and leaves the semantic reimplementation and its drift in place.

## Consequences

- The capability model applies to remote callers for the first time. Operator-classified operations become reachable by their intended client without widening the ungated path.
- Schema drift between orbit and natively-served remote clients becomes structurally impossible: contract negotiation at initialize hard-fails mismatched builds.
- The reimplemented proxy layer becomes deletable rather than portable. Its recurring per-release maintenance disappears with it.
- **Cost:** per-session construction is a correctness precondition, not an optimisation. Shipping the transport without it silently cross-contaminates workspace selection between concurrent clients — a failure that presents as wrong data rather than as an error.
- **Cost:** version lockstep becomes load-bearing. A server older than its client fails negotiation outright rather than degrading, so server deployment and client upgrade must be coordinated from this point on.
- **Cost:** the server process gains a second protocol surface, and its availability now matters to work dispatch rather than only to the UI. Its failure modes are correspondingly more expensive.
- The ungated HTTP API remains a bypass. This decision reduces what depends on it; it does not close it. That remains separately owned.
## Context

The coupled Host Registry and MCP Bridge implementation now spans registry identity/catalog/cache in `orbit-registry`, active registry SQL in `orbit-store`, registry-aware runtime and tool dispatch in `orbit-core`, canonical remote tools in `orbit-tools`, remote protocol hooks in `orbit-mcp`, and broker/hub/link/registration composition in `orbit-cli`. The previously proposed answer was an additional horizontal `orbit-mcp-broker` crate above those layers. That would remove code from the CLI but preserve the seven-crate change tax for every later remote feature.

The real alternatives are: keep the horizontal layers and add the broker crate; absorb all RMCP infrastructure into the feature; or create one vertical remote feature crate while retaining small generic infrastructure kernels. The first preserves the coupling problem, while the second makes reusable RMCP transport inseparable from Orbit's host-registry policy.

## Decision

Rename and expand `orbit-registry` into one internal vertical feature crate, `orbit-remote`. It owns host identity, workspace roles/catalog/cache, registry services, active registry SQL and namespaced feature migrations, trusted hub configuration, broker/hub/link/registration composition, remote tool definitions and handlers, remote-specific MCP contracts and client behavior, and local graph/learning MCP composition.

The dependency direction is `orbit-cli|orbit-dashboard -> orbit-remote -> {orbit-core, orbit-store, orbit-tools, orbit-mcp, orbit-graph, orbit-graph-extract, orbit-common}`. Core, Store, Tools, MCP, and Graph do not depend back on Remote. Core exposes registry-free runtime bindings, execution-environment snapshots, routine-placement projections, and the transport-independent checkoutless `HubCoordinationExecutor`; Remote calls that executor from its hub/broker orchestration without injecting remote tools into Core. Store exposes generic pooled-read, transaction, and namespaced feature-migration capabilities. Tools retains its generic builtin registry, while Remote composes those definitions with feature-owned discovery and graph definitions. MCP retains only reusable RMCP framing, schema/name translation, stdio transport, raw client primitives, and generic extension/composition hooks.

Keep the existing config-resolved global `orbit.db`. Shipped global migrations v5, v6, and v8 remain immutable compatibility shims and v7 remains Store-owned audit schema. Global v9 introduces only the generic feature-schema ledger; `orbit-remote` feature migration v1 validates and adopts the existing registry tables without copying, renaming, or rewriting data. All future remote schema changes advance the Remote feature ledger rather than editing Store's domain migration list.

This decision supersedes ADR-0235's narrower `orbit-registry -> orbit-store` domain boundary and replaces the unimplemented `orbit-mcp-broker` proposal.

## Consequences

- Later registry, knowledge-routing, placement, runner, and hub-link work has one owning feature crate; shared crates change only when a genuinely generic contract or infrastructure seam changes.
- `orbit-cli` and `orbit-dashboard` become consumers of a stable Remote facade instead of composition owners, and `orbit-core` no longer imports registry types or services.
- Registry persistence keeps its existing database, table names, transaction boundaries, and data while active SQL and behavior tests move beside the domain.
- `orbit-mcp` remains reusable and acyclic; graph, learning, hub contract, registration, and routing policy are composed by Remote.
- Historical migrations cannot be deleted after ownership moves; Store permanently retains the frozen bootstrap shims required to open old and fresh databases.
- Cost: this is a larger atomic refactor than a broker-only move. It requires neutral Core factories/providers, one generic Store migration seam, MCP composition hooks, and coordinated test relocation before the crate rename can compile without a dependency cycle.
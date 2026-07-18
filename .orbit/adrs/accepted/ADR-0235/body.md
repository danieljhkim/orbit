## Context

C3 left strict host identity, the logical workspace catalog, the satellite cache, and the store-backed registry service in orbit-core while the existing orbit-registry crate exposed an unused opaque-byte replication and merge model. The real alternatives were to keep that domain in orbit-core, retain a generic replication substrate alongside it, or establish a dedicated one-way registry domain boundary.

## Decision

Repurpose orbit-registry as the machine/workspace registry domain crate. It owns host identity, local workspace catalog/checkouts/roles, registry-cache semantics, and the store-backed HostRegistryService; orbit-core depends on it and temporarily re-exports compatibility surfaces. orbit-store remains the only owner of SQL, migrations, revision advancement, and transactional snapshot queries, and it must never depend on orbit-registry. The opaque replicated Registry/Replica/merge/transport model is retired because v1 has one singular coordination hub and no replicated registry writers.

## Consequences

- The intended dependency direction is orbit-core -> orbit-registry -> orbit-store -> orbit-common, with orbit-core also retaining its direct orbit-store edge.
- Runtime execution-profile construction, catalog validation, and ship-closure hashing stay in orbit-core; shared DTOs stay in orbit-common.
- Compatibility re-exports let current callers migrate imports incrementally without preserving a second domain implementation.
- Cost: orbit-registry is no longer a consumer-agnostic leaf and now compiles the store layer; reversing this boundary would require moving the domain again or introducing a cycle-prone abstraction.
## Context
The ADR-artifact proposal touches `orbit-common`, `orbit-store`, `orbit-tools`, `orbit-cli`, and the entire `docs/design/*` corpus. Shipping it as a single v1 change would block other work and rush the migration. The existing `4_decisions.md` markdown pattern is functional today; the problems it causes are growth-rate problems, not correctness problems.

## Decision
v1 ships this folder as docs-only. No `orbit.adr.*` code, no migration tooling, no CONVENTIONS.md change. v2 ships the store, tools, migration, and the convention update as a coordinated sequence of tasks.

## Consequences

- The design is captured while context is fresh and can be cross-reviewed before any code lands.
- Until v2 ships, decisions about this feature live in this `4_decisions.md` — recursively in the form the proposal replaces. Acceptable bootstrap cost.
- Other feature folders continue accumulating markdown ADRs that will need migration; the corpus grows in the meantime.
- Cost: the migration sweep gets bigger every week v2 is deferred. The trim-as-you-touch rule from [CLAUDE.md](../../../CLAUDE.md#design-docs) does *not* apply here — leads should not pre-migrate to a store that doesn't exist.

---

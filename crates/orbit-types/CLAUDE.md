# orbit-types

Project instructions for the lowest internal contract crate.

## One job

Shared **data contracts**: structs, enums, serde shapes, pure constructors,
normalization, lifecycle predicates, and narrow domain errors. Nothing else.

`orbit-types` has **zero Orbit dependencies** and is the only crate that may be
depended on by every other crate. It performs no filesystem, process,
environment, database, network, logging, or tracing work — if a change here
needs `std::fs`, `std::process`, `std::env`, `rusqlite`, or `tracing`, the
behavior belongs one layer up in `orbit-common` and only its *shape* belongs
here.

## Domain-qualified modules, never a grab-bag

Every item lives under exactly one domain module: [`identity`](src/identity),
[`policy`](src/policy), [`record`](src/record), [`resource`](src/resource),
[`task`](src/task), [`telemetry`](src/telemetry), [`tool`](src/tool),
[`workflow`](src/workflow), [`workspace`](src/workspace).

`OrbitId` is the **only** crate-root primitive. Do not add a second one; pick
the domain it belongs to instead. A new top-level module means a genuinely new
domain, not a convenient home for something that did not fit.

Each domain module follows the same shape — see
[`identity/mod.rs`](src/identity/mod.rs) or [`task/mod.rs`](src/task/mod.rs):

- Submodules are **private** (`mod actor;`), never `pub mod`.
- `mod.rs` is declarations plus an explicit `pub use` list. The re-export list
  *is* the module's public surface; adding a type without listing it there
  keeps it unreachable on purpose.
- One `error.rs` per domain holding a narrow `thiserror` enum
  (`TaskError`, `IdentityError`, `RecordError`, …) re-exported from `mod.rs`.
  Workspace-wide `OrbitError` lives in `orbit-common` and must not appear here.
- `#[cfg(test)] mod tests;` pointing at a sibling `tests/` directory that
  mirrors source filenames ([`test_layout.md`](../../docs/design-patterns/test_layout.md)).

Choosing between neighbouring domains is a real decision, not a coin flip:
`record` holds durable authored artifacts (ADR, friction, event, audit
record), while `telemetry` holds measurement of runs (invocation traces, audit
events, token pricing, metrics). Put a new type where its *lifecycle* belongs.

## Serde shapes are persisted contract

Types here are serialized into task bundles, YAML definitions, SQLite columns,
and the MCP wire. Renaming a field or changing a `#[serde]` attribute is a data
migration, not a rename — check for a reader in `orbit-store` (bundle/driver
code and its `migration` ledger) before touching a shape, and keep
`*_SCHEMA_VERSION` constants and their guards in step.

The crate is tier **stable** in [`ARCHITECTURE.md`](../../ARCHITECTURE.md);
breaking a public shape needs a deliberate decision, not a drive-by cleanup.

## The `clap` feature is presentation-only

`clap` is an optional dependency used exclusively through
`#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]` and `value(...)`
attributes so CLI enums parse from argv without the CLI redefining them. Do not
grow it into `Args`/`Parser` derives, help text, or any other command surface —
that is `orbit-cli`'s job. A default build of this crate must not link clap.

# Crate-Boundary Error Translation

Each crate defines its own typed error (`thiserror`-derived) for internal use; `OrbitError` in `orbit-common` is the workspace-public error surface. A single translation function — `*_error_to_orbit` — lives next to the typed error and is called at every cross-crate boundary via `.map_err(...)`.

```rust
// In crate-foo:
#[derive(thiserror::Error, ...)]
pub struct FooError { pub kind: String, pub reason: String }

pub fn foo_error_to_orbit(error: FooError) -> OrbitError {
    if error.kind == "foo_invalid" {
        OrbitError::InvalidInput(error.reason)
    } else {
        OrbitError::Execution(error.to_string())
    }
}

// At any caller that returns OrbitError:
foo::do_thing(...).map_err(foo_error_to_orbit)?
```

The principle: internal code propagates the rich typed error so callers can match on variants; once the error crosses the crate boundary it's collapsed to `OrbitError` so the workspace's public surface stays uniform.

## When to reach for it

- **You're adding a new crate.** Define a typed error there. Export a `*_error_to_orbit` translator. Don't `pub use OrbitError` as your crate's error type — that couples your internals to the workspace surface.
- **Your crate already has its own error and now needs to be called from a crate that returns `OrbitError`.** The boundary is `.map_err(translator)?`, never an ad-hoc `OrbitError::Execution(other_err.to_string())` at the callsite.
- **The same `OrbitError` variant should be produced from many translation sites.** Centralizing the kind→variant mapping in one function keeps the public error surface coherent.

## When NOT to

- **Within a single crate.** Use the typed error directly. Translating mid-crate discards information you might want at the next layer.
- **You don't have a typed error yet.** A thin wrapper crate producing `OrbitError` directly is fine; introduce a typed error only when you have enough variants that matching on them adds value.
- **The "translation" is `OrbitError::from(other_err.to_string())`.** Stringifying loses the kind. If that's all your translator does, you don't need one — write the one-line `.map_err` at the boundary.

## Reference: `KnowledgeError` → `OrbitError`

A thiserror struct with a string `kind` field, and a translator that maps a known kind to a specific `OrbitError` variant and dumps the rest into the generic bucket. From `crates/orbit-knowledge/src/error.rs:6`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{kind}: {reason}")]
pub struct KnowledgeError {
    pub kind: String,
    pub reason: String,
}

impl KnowledgeError {
    pub(crate) fn knowledge_unavailable(reason: impl Into<String>) -> Self { /* ... */ }
    pub(crate) fn invalid_data(reason: impl Into<String>) -> Self { /* ... */ }
    pub(crate) fn io(reason: impl Into<String>) -> Self { /* ... */ }
}
```

The translator at `crates/orbit-knowledge/src/commands/mod.rs:108`, re-exported at the crate root (`crates/orbit-knowledge/src/lib.rs:49`):

```rust
pub fn knowledge_error_to_orbit(error: KnowledgeError) -> OrbitError {
    if error.kind == "knowledge_invalid" {
        OrbitError::InvalidInput(error.reason)
    } else {
        OrbitError::Execution(error.to_string())
    }
}
```

Used at every cross-crate edge — e.g. `crates/orbit-tools/src/builtin/orbit/knowledge/show.rs:53`:

```rust
let pack = orbit_knowledge::commands::show::pack(...)
    .map_err(super::knowledge_error_to_orbit)?;
```

Patterns to copy:

- **Translator lives in the source crate, next to the error.** Not in `orbit-common`, not in each caller. The crate that *defined* `FooError` owns the kind→variant mapping. Re-export at the crate root so callers can `use crate_foo::foo_error_to_orbit;`.
- **Discriminator field drives the mapping.** A typed `kind: String` (or an enum, equivalently) lets the translator branch without exposing internal `thiserror` variants to consumers.
- **Constructors are `pub(crate)`.** Outside callers receive `KnowledgeError` from existing APIs; they never construct one. This keeps the kind set narrow and meaningful.
- **One named match per surfaced variant; everything else passes through.** "`knowledge_invalid` → `InvalidInput`, default → `Execution`" is the right granularity — name the kinds callers will actually branch on, dump the rest into the generic bucket.
- **`.map_err(translator)?`, not `.map_err(|e| translator(e))?`.** The translator's signature is `FnOnce(E) -> OrbitError`, so the bare path works as a closure. The shorter form reads better at boundary sites.

Use this shape for every new crate in the workspace per the architecture diagram in `CLAUDE.md`. A new typed error should land in the same PR as its translator.

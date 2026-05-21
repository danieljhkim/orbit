---
type: pattern
summary: "Per-Module tests/ Directory Layout"
---
# Per-Module tests/ Directory Layout

Unit tests for each module live in a sibling `tests/` directory rather than inline at the bottom of the source file or in a `*_tests.rs` neighbour. The source file declares the test module; the bodies live in files named per concern. Submodules can read parent privates, so tests retain crate-internal visibility while production directories stay free of `#[cfg(test)]` clutter.

```text
src/command/
  skill.rs                       # declares: #[cfg(test)] mod tests;
  skill/
    tests/
      mod.rs                     # `mod parse; mod render; mod install;`
      parse.rs                   # one submodule per concern
      render.rs
      install.rs
```

```rust
// src/command/skill.rs
pub(crate) fn install_skill(...) -> Result<...> { ... }

#[cfg(test)]
mod tests;                       // body lives in skill/tests/mod.rs
```

```rust
// src/command/skill/tests/mod.rs
mod parse;
mod render;
mod install;
```

```rust
// src/command/skill/tests/install.rs
use super::super::install_skill;  // parent module's pub(crate) item

#[test]
fn installs_skill_into_workspace() { ... }
```

The principle: keep production directory listings free of test artefacts, split test bodies by concern from day one (no later "this `tests.rs` got too big" refactor), and test only the module's exposed surface so untestability surfaces as design pressure instead of being absorbed by private-helper probes.

## When to reach for it

- **You're adding a new module that needs tests.** Use this layout from the first file. Don't start with inline `#[cfg(test)] mod tests { ... }` "for now" — the migration cost is the same as doing it right once.
- **A module's inline test block has grown past ~50 lines or covers multiple concerns.** Split it now; future authors will copy whatever shape they find.
- **A test wants to verify behaviour at a `pub`/`pub(crate)` boundary.** That is exactly what this layout targets.

## When NOT to

- **You're tempted to test a private helper directly.** Don't widen visibility to `pub(crate)` just to test it — restructure so the seam is at a deliberate boundary, or accept that the helper is covered transitively through its public caller. Visibility widening that exists only for test access is a design smell.
- **You're writing an integration test that exercises the crate's public API end-to-end.** Use crate-root `tests/<name>.rs` — Cargo compiles each as a separate binary against the crate's public surface, which is exactly what integration tests want.
- **The module has zero non-trivial logic worth a unit test.** Don't scaffold `tests/mod.rs` for the sake of uniformity; an empty submodule with one smoke test is noise.

## Reference: `orbit-engine::activity_job::job_executor`

The transitional layout uses sibling `*_tests.rs` files (`audit_tests.rs`, `fanout_tests.rs`, `loop_tests.rs`, …) included from `job_executor/mod.rs` via `#[cfg(test)] mod tests;` indirection. The target layout collapses these into `job_executor/tests/{audit,fanout,loop,…}.rs` declared from `job_executor/tests/mod.rs`, with the parent `mod.rs` keeping the same `#[cfg(test)] mod tests;` declaration. Migration is tracked under [ORB-00219] (convention) plus per-crate sibling tasks.

## Migration recipe

For each source file with inline `#[cfg(test)] mod tests { ... }`:

1. Identify concerns inside the block (parse, render, error paths, etc.). One submodule per concern.
2. Create `src/<module>/tests/` and `src/<module>/tests/mod.rs` declaring each submodule.
3. Move test bodies into the per-concern files. Replace `use super::*;` with explicit `use super::super::<item>;` imports through the exposed surface.
4. Replace the inline block in the source file with `#[cfg(test)] mod tests;`.
5. If any test required a private item, decide deliberately: widen the item's visibility with a comment explaining why, or restructure the test against a public seam.
6. `cargo test -p <crate>` must pass.

For each sibling `*_tests.rs` file:

1. Move the file into `<parent>/tests/<topic>.rs` (drop the `_tests` suffix; the directory already says "tests").
2. Add the submodule to `<parent>/tests/mod.rs`.
3. Update any `use super::*;` to walk one more level up.
4. `cargo test -p <crate>` must pass.

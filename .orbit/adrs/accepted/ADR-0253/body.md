## Context
 ADR-0209 bearing 1 describes one operation table holding both the
serializable definition and its handler. Orbit's layering makes that
unreachable: every surface (`orbit-tools`, `orbit-cli`, `orbit-dashboard`) must
read the definition, so it has to live at or below `orbit-common`; handlers need
`&OrbitRuntime`, which lives in `orbit-core`, well above it. Co-locating them
would either drag the runtime into the leaf crate or lift the specs above the
surfaces that consume them — both new dependency edges that `ARCHITECTURE.md`
forbids.


## Decision
 Split the table across the two crates and join it with the noun's
typed verb enum: `&'static [OperationSpec<V>]` in `orbit-common`, an exhaustive
`match` on `V` in `orbit-core`. `V` is the only thing both halves share, and
because both the spec lookup and the handler dispatch are exhaustive matches, a
verb that is declared but not implemented fails to compile.


## Consequences


- Compile-time completeness across a crate boundary with no codegen, no trait
  object, and no runtime registration phase.
- Adding a verb breaks the build in exactly two known places, which is a usable
  to-do list rather than a silent gap.
- Future noun migrations should adopt this shape rather than re-attempting
  co-location; ADR-0209's stored body records the correction.
- If ADR-0209 bearing 2 (knowledge/execution split) moves knowledge handlers
  below the surfaces, the halves could merge and this ADR would be superseded.
- Cost: "the operation table" is now two files in two crates, so a reader
  looking for an operation's behavior must follow the verb enum to find the
  handler — the definition alone does not tell you what happens.
---
title: Operations as Data — Decisions
owner: claude
last_updated: 2026-07-26
status: Accepted
feature: operations-as-data
doc_role: decisions
type: design
summary: ADR log for the operations-as-data registry — the split spec/handler table, what stayed hand-written, and the touch-it-move-it ratchet.
tags: [operations-as-data, architecture, adr-0209]
paths: ["crates/orbit-common/src/operation.rs", "crates/orbit-common/src/friction/**"]
related_features: [operations-as-data]
related_artifacts: [ORB-10358, ADR-0209, ADR-0253, ADR-0254, ADR-0255]
---

# Operations as Data — Decisions

ADR log for operations-as-data. Entries are append-only and ordered by ascending
global ID. The store owns ID, status, owner, and links; this file is the
long-form narrative keyed on that same ID. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the rules.

The parent bearing is **ADR-0209** (north-star: operations as data behind an
operation registry), whose stored body now carries the friction pilot outcome and
the ratchet.

## ADR-0253 — Split spec/handler table joined by a typed verb enum

**Status:** Accepted · 2026-07 · [ORB-10358]

**Context.** ADR-0209 bearing 1 describes one operation table holding both the
serializable definition and its handler. Orbit's layering makes that
unreachable: every surface (`orbit-tools`, `orbit-cli`, `orbit-dashboard`) must
read the definition, so it has to live at or below `orbit-common`; handlers need
`&OrbitRuntime`, which lives in `orbit-core`, well above it. Co-locating them
would either drag the runtime into the leaf crate or lift the specs above the
surfaces that consume them — both new dependency edges that `ARCHITECTURE.md`
forbids.

**Decision.** Split the table across the two crates and join it with the noun's
typed verb enum: `&'static [OperationSpec<V>]` in `orbit-common`, an exhaustive
`match` on `V` in `orbit-core`. `V` is the only thing both halves share, and
because both the spec lookup and the handler dispatch are exhaustive matches, a
verb that is declared but not implemented fails to compile.

**Consequences.**

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

## ADR-0254 — Renderers and HTTP routes stay hand-written

**Status:** Accepted · 2026-07 · [ORB-10358]

**Context.** Once verbs are data, the obvious next step is to make the rest of
each surface data too: CLI output formatting and dashboard REST routes were both
candidates. Both would have grown the registry and both were rejected during the
friction pilot.

**Decision.** The spec declares *which* rendering a verb wants (`CliRender`) but
not how to render; the friction record and table printers stay in `orbit-cli` and
know friction field names. Dashboard route shapes, serde request bodies, and
HTTP-specific defaults stay hand-written in `orbit-dashboard`, which takes only
tool names and parameter names from the registry.

**Consequences.**

- The registry stays a description of *operations*, not of presentation, which
  keeps it readable and keeps its blast radius to contract.
- A REST path remains an interface design decision made per route, not a
  mechanical consequence of adding a verb.
- Adding a friction verb that should be reachable over HTTP is still a two-place
  change (registry + route).
- A noun whose output has a genuinely new shape needs a new `CliRender` variant
  plus a renderer — also two places.
- Cost: the dashboard is only partially derived, so it remains possible to add a
  verb and forget the route entirely; nothing fails, the verb is simply absent
  from the web UI, and no test catches it.

## ADR-0255 — Freeze the pre-migration surface as fixtures before migrating

**Status:** Accepted · 2026-07 · [ORB-10358]

**Context.** The pilot's hard requirement was that CLI argv/output and MCP tool
schemas stay wire-compatible. Derived clap help output is only byte-stable
because the adapter reproduces `#[derive(Args)]`'s conventions (arg id,
SCREAMING_SNAKE value name, declaration-order display) — a correspondence
nothing in the type system enforces. Verifying it after the fact, from the
migrated code, proves nothing.

**Decision.** Capture the pre-migration surface before writing any migration
code, and commit it as test fixtures. For friction: `orbit friction [<verb>]
--help` for all eight pages, captured from the binary built at the prior commit
and frozen under `crates/orbit-cli/src/command/tests/friction_help/`, asserted
via `include_str!`. The already-in-tree `mcp_tools_list.json` snapshot serves the
same role for MCP, where an empty `git diff` is the proof.

**Consequences.**

- "Wire compatible" became a checkable claim rather than a review assertion; the
  friction migration reproduces all eight help pages and the MCP snapshot
  byte-for-byte.
- The fixtures keep working after the migration as a regression guard on the
  derived surface, including across clap upgrades.
- Every future noun migration must do this first — the cookbook makes it Step 0.
- Cost: the fixtures encode incidental formatting (clap's global-arg placement,
  column alignment), so an intentional, approved CLI change now requires
  re-blessing files whose diff is mostly noise — and the fixture must be
  distinguished from a genuine regression by a human reading the PR.

## Task References

- [ORB-10358] — piloted ADR-0209 bearing 1 on the friction noun; produced the
  split table, the derived adapters, and the frozen-surface method.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

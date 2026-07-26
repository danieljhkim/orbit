## Context
 The pilot's hard requirement was that CLI argv/output and MCP tool
schemas stay wire-compatible. Derived clap help output is only byte-stable
because the adapter reproduces `#[derive(Args)]`'s conventions (arg id,
SCREAMING_SNAKE value name, declaration-order display) — a correspondence
nothing in the type system enforces. Verifying it after the fact, from the
migrated code, proves nothing.


## Decision
 Capture the pre-migration surface before writing any migration
code, and commit it as test fixtures. For friction: `orbit friction [<verb>]
--help` for all eight pages, captured from the binary built at the prior commit
and frozen under `crates/orbit-cli/src/command/tests/friction_help/`, asserted
via `include_str!`. The already-in-tree `mcp_tools_list.json` snapshot serves the
same role for MCP, where an empty `git diff` is the proof.


## Consequences


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
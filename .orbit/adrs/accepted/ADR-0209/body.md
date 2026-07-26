## Context
Orbit's four consumer surfaces — the CLI (`orbit-cli`), MCP (`orbit-mcp`), the web dashboard (`orbit-dashboard`), and the in-runtime agent tool hosts — are four hand-wired adapter layers over the same underlying operations, so every new operation is plumbed by hand up to four times. The same shape keeps constraining refactors: inherent `impl OrbitRuntime` methods plus the orphan rule forced the ORB-10016 / ADR-0203 orbit-cmd extraction to leave the runtime-entangled command groups behind in orbit-core as documented residuals, and the same wall shelved the docs+search pluginization (docs/design/orbit-docs-plugin/1_scope.md — pending commit). Repeated point refactors treat symptoms; the missing piece is a recorded long-term bearing that future refactors steer by. Real alternatives existed: keep the status quo and continue paying per-surface wiring, or mandate a big-bang plugin/microkernel rewrite.

## Decision
Record five bearings as orbit's north star. This is an **incremental bearing, not a rewrite mandate**: no code changes are required by this ADR, and existing code is not wrong for predating it.

1. **Operations as data, not inherent methods.** Every orbit operation is eventually defined as a serializable request/response pair with a handler registered in an operation table. The four consumer surfaces become derived adapters over that registry instead of four hand-wired layers, and the recurring inherent-impl/orphan-rule constraint (ADR-0203 residuals; the shelved docs+search pluginization) dissolves because handlers are registry entries, not inherent methods on `OrbitRuntime`.
2. **Knowledge/execution split.** Orbit is two products — a knowledge store (tasks, learnings, ADRs, docs) and an execution engine (activities, jobs, agent providers) — glued by one runtime. Bearing: two systems sharing only a kernel (IDs, errors, audit), mirroring the constellation split (polaris = knowledge, worker = execution).
3. **Events over side-effects.** The task-mutation → semantic-index coupling becomes a transactional SQLite outbox consumed by the indexer, replacing the lossy in-process `EmbedWorker` enqueue (best-effort batches, drops on queue-full, debug-level failure logging).
4. **One retrieval trait, two backends.** orbit-search (workspace-local) and sextant (constellation-wide) become deployment choices behind one retrieval interface, dissolving the two-stack question.
5. **Crates follow build boundaries, not taxonomy.** Crate splits are justified by compile-graph and dependency-direction needs, not by conceptual category. Explicitly kept as-is under this bearing: the ADR-005 companion-subprocess packaging pattern (docs/design/orbit-search/4_decisions.md ADR-005, global ADR-0117), the YAML+SQLite layered store in orbit-store, and the stability-tier markers (ARCHITECTURE.md §Stability tiers).

**Adoption model.** Incremental and opportunistic: when a new surface is added or an existing command group is touched for other reasons, move that slice to request/response + registry then. Future ADRs should cite this bearing when steering by it, or supersede it if the bearing itself changes.

**Alternatives rejected.** (a) *Status quo as the implicit bearing*: keeps charging up to 4× adapter wiring per operation and guarantees the next boundary refactor hits the same inherent-impl/orphan-rule wall with no recorded direction — the cost that motivated this ADR. (b) *Big-bang registry rewrite*: months of churn across every surface with no incremental payoff and high regression risk in a codebase that ships continuously; rejected in favor of the touch-it-move-it model.

## Consequences
- Future architecture ADRs have a fixed point to cite or supersede; "which direction were we going?" is answerable from the store.
- Slices touched after this ADR should trend toward request/response + registry; reviewers can ask "why not the registry shape?" when a new hand-wired adapter appears.
- The knowledge/execution split (bearing 2) gives crate and module moves a destination: kernel-shared code is IDs/errors/audit only.
- The transition is unbounded by design, so registry-shaped and inherent-method slices will coexist indefinitely; the registry idiom is the tie-breaker, not a deadline.
- No single code anchor; convention enforced via review.
- Cost: two coexisting idioms during the (unbounded) incremental transition — readers must recognize both the registry shape and the legacy inherent-method shape — and request/response indirection adds per-operation boilerplate (a request type, a response type, a registration) compared to calling an inherent method directly.

## Pilot outcome — bearing 1, friction noun [ORB-10358]

**Status of bearing 1:** piloted and proven on one noun. Not yet applied to the
other three hand-copied nouns; that is the ratchet below, not a backlog item.

**What shipped.** Every friction verb (`add`, `list`, `show`, `stats`, `tags`,
`update`, `resolve`) is declared exactly once as an `OperationSpec` in
`orbit_common::friction::operations`. All four surfaces are now derived adapters
over that table: `orbit-tools` builds each `ToolSchema` and MCP exposure policy
from the spec (seven hand-written `Tool` impls deleted), `orbit-cli` builds the
clap subcommand tree and the tool input from the spec (seven `Args` structs and
seven `Execute` impls deleted), `orbit-dashboard` takes its tool names and
parameter names from the registry, and `orbit-core` holds the handler half of
the table keyed on `FrictionVerb`. `OrbitBuiltinAction`'s seven `Friction*`
variants collapsed to one `Friction(FrictionVerb)`.

**Contract stability was proven, not asserted.** `crates/orbit-cli/tests/snapshots/mcp_tools_list.json`
is byte-unchanged, and `orbit friction [<verb>] --help` was captured from the
pre-migration binary and frozen as fixtures under
`crates/orbit-cli/src/command/tests/friction_help/`; the derived CLI reproduces
all eight help pages byte-for-byte.

**The layering correction to the bearing.** Bearing 1 as written implies one
table holding both the operation definition and its handler. That is not
achievable while handlers need `&OrbitRuntime`, which lives well above the leaf
crate every surface can read. The working shape is a **split table joined by a
typed verb enum**: the spec table is `&'static [OperationSpec<V>]` in
`orbit-common`, the handler table is one exhaustive `match` on `V` in
`orbit-core`, and the compiler rejects a verb that has a spec but no handler.
Future noun migrations should adopt that shape rather than trying to co-locate
handlers with specs.

**What did not become data, deliberately.** Response rendering stayed per-noun
(the CLI's friction table/record printers know friction field names), and
dashboard route shapes stayed hand-written — a REST path is an HTTP design
choice, not a property of the verb. The registry declares *which* rendering a
verb wants; the renderer itself is presentation.

**Touch-it-move-it ratchet.** The next feature that touches any hand-copied noun
migrates that noun to the registry as part of that work. "Touches" means adding
or removing a verb, changing a verb's parameters, or changing how any surface
wires that noun — not an unrelated edit inside an existing handler. A reviewer
seeing a new hand-wired adapter for a noun that could have been migrated should
ask for the migration or an explicit reason in the PR. Migration cookbook, with
the friction diff as the worked example: `docs/design/operations-as-data/`.

**Costs the pilot actually surfaced.**
- The registry is a wall of literal strings, and every one of them is shipped
  contract. This trades scattered-but-local duplication for centralized
  contract, which is the point, but it makes the registry file a high-blast-radius
  edit.
- Two descriptions per parameter where MCP and CLI wording legitimately differ
  (`show`'s id is `friction ID` over MCP and `Friction record id, e.g.
  F2026-05-001` on the CLI). The spec models this rather than forcing them to
  converge, because converging them would have been a wire change.
- `clap::Arg::value_name` takes only `&'static str`, so building a clap surface
  from runtime data requires interning. The adapter leaks a bounded number of
  short strings for the process lifetime.
- Derived clap `--help` output is only byte-stable because the adapter
  reproduces `#[derive(Args)]`'s conventions exactly (arg id, SCREAMING_SNAKE
  value name, declaration-order display). Any future noun migration must freeze
  its pre-migration help output as fixtures before starting, as this one did.
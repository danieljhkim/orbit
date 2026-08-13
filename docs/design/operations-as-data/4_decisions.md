---
title: Operations as Data — Decisions
owner: claude
last_updated: 2026-08-13
last_validated: 2026-08-09
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

> **Retired learning clauses:** [ORB-10736] / [ADR-0359] removed the native
> project-learning CLI and tool operations. Learning-specific examples in
> earlier entries are retained as historical context only.

Ordered pointer index for operations-as-data ADRs. The store owns each title,
status, and authoritative narrative; print a body with `orbit tool run
orbit.adr.show --input '{"id":"ADR-NNNN"}'`. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict)
for the rules.

The parent bearing is **ADR-0209** (north-star: operations as data behind an
operation registry), whose stored body now carries the friction pilot outcome and
the ratchet.

- **ADR-0253 — Split spec/handler table joined by a typed verb enum** — Accepted.
- **ADR-0254 — Renderers and HTTP routes stay hand-written** — Accepted.
- **ADR-0255 — Freeze the pre-migration surface as fixtures before migrating** — Accepted.

## ADR-0253 — Split spec/handler table joined by a typed verb enum

**Status:** Accepted · 2026-07-26 00:55:47.755882Z · [ORB-10358]
**Owner:** claude
**Created:** 2026-07-26 00:55:37.563417Z
**Last updated:** 2026-07-26 00:55:47.755882+00:00
**Related features:** `operations-as-data`
**Tags:** `operations-as-data`, `architecture`, `adr-0209`
**Paths:** `crates/orbit-common/src/operation.rs`, `crates/orbit-common/src/friction/**`

### Context
 ADR-0209 bearing 1 describes one operation table holding both the
serializable definition and its handler. Orbit's layering makes that
unreachable: every surface (`orbit-tools`, `orbit-cli`, `orbit-dashboard`) must
read the definition, so it has to live at or below `orbit-common`; handlers need
`&OrbitRuntime`, which lives in `orbit-core`, well above it. Co-locating them
would either drag the runtime into the leaf crate or lift the specs above the
surfaces that consume them — both new dependency edges that `ARCHITECTURE.md`
forbids.


### Decision
 Split the table across the two crates and join it with the noun's
typed verb enum: `&'static [OperationSpec<V>]` in `orbit-common`, an exhaustive
`match` on `V` in `orbit-core`. `V` is the only thing both halves share, and
because both the spec lookup and the handler dispatch are exhaustive matches, a
verb that is declared but not implemented fails to compile.


### Consequences


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

**Status:** Accepted · 2026-07-26 00:55:47.967899Z · [ORB-10358]
**Owner:** claude
**Created:** 2026-07-26 00:55:37.797226Z
**Last updated:** 2026-07-26 00:55:47.967899Z
**Related features:** `operations-as-data`
**Tags:** `operations-as-data`, `architecture`, `adr-0209`
**Paths:** `crates/orbit-common/src/operation.rs`, `crates/orbit-common/src/friction/**`

### Context
 Once verbs are data, the obvious next step is to make the rest of
each surface data too: CLI output formatting and dashboard REST routes were both
candidates. Both would have grown the registry and both were rejected during the
friction pilot.


### Decision
 The spec declares *which* rendering a verb wants (`CliRender`) but
not how to render; the friction record and table printers stay in `orbit-cli` and
know friction field names. Dashboard route shapes, serde request bodies, and
HTTP-specific defaults stay hand-written in `orbit-dashboard`, which takes only
tool names and parameter names from the registry.


### Consequences


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

**Status:** Accepted · 2026-07-26 00:55:48.187121Z · [ORB-10358]
**Owner:** claude
**Created:** 2026-07-26 00:55:38.009856Z
**Last updated:** 2026-07-26 00:55:48.187121Z
**Related features:** `operations-as-data`
**Tags:** `operations-as-data`, `architecture`, `adr-0209`
**Paths:** `crates/orbit-common/src/operation.rs`, `crates/orbit-common/src/friction/**`

### Context
 The pilot's hard requirement was that CLI argv/output and MCP tool
schemas stay wire-compatible. Derived clap help output is only byte-stable
because the adapter reproduces `#[derive(Args)]`'s conventions (arg id,
SCREAMING_SNAKE value name, declaration-order display) — a correspondence
nothing in the type system enforces. Verifying it after the fact, from the
migrated code, proves nothing.


### Decision
 Capture the pre-migration surface before writing any migration
code, and commit it as test fixtures. For friction: `orbit friction [<verb>]
--help` for all eight pages, captured from the binary built at the prior commit
and frozen under `crates/orbit-cli/src/command/tests/friction_help/`, asserted
via `include_str!`. The already-in-tree `mcp_tools_list.json` snapshot serves the
same role for MCP, where an empty `git diff` is the proof.


### Consequences


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

## ADR-0260 — Capability chokepoint for destructive operations outside MCP

**Status:** Accepted · 2026-07-26 21:49:30.348935Z · [ORB-10453]
**Owner:** claude
**Created:** 2026-07-26 21:35:37.651977Z
**Last updated:** 2026-07-26 21:51:33.078323Z
**Tags:** `cli`, `mcp`, `capabilities`, `privilege-model`, `safety`
**Paths:** `crates/orbit-common/src/authorization.rs`, `crates/orbit-core/src/runtime/authorization.rs`, `crates/orbit-core/src/runtime/tool_exec.rs`, `crates/orbit-cli/src/main.rs`, `crates/orbit-cli/src/command/operation.rs`

### Context

Orbit had a capability model — `McpCapability::{Agent, Operator, Runner}` with `McpToolPolicy`, granted per server via `orbit mcp serve --capabilities` — and it governed exactly one surface. Three gaps followed.

The CLI dispatch path had no capability check at all. `register_inactive` hid destructive builtins from MCP listing, but hiding is advertisement, not enforcement: CLI subcommands reached those same tools through the admin `runtime.run_tool` bypass, and `orbit workspace teardown`, `orbit audit prune`, and `orbit learning prune` destroy things without touching a tool at all. And `task.approval.required_for_agent` / `task.approval.delegate_approval` were registered config keys threaded builder → context → runtime with no enforcement call site anywhere, with `OrbitError::TaskApprovalRequired` mapped in error formatting but never constructed.

The net effect was that no single place decided "may this caller perform this operation", so any per-command guard could be routed around by the next entry point.

This is an **accident guard, not a security boundary**. Agents on a development box run as the same OS user and can bypass Orbit entirely with git, the filesystem, or a direct write to the data root. Any design implying otherwise — in particular a credential or password — is explicitly rejected: it buys no protection and invites relaxing the surrounding rails on the strength of a boundary that does not exist. The goal is that unintended destruction fails loudly and leaves a record.

### Decision

1. **Extend the existing capability model; do not add a second one.** `McpCapability` is the vocabulary for every surface. MCP becomes one consumer of the model rather than its owner.

2. **Declare governed operations once, as data.** `orbit_common::authorization::GOVERNED_OPERATIONS` is a const registry of `{ id, surface, allowed: &[McpCapability], rationale }`. Call sites name an operation, never a capability; the requirement is resolved from the registry. It lives in the leaf crate for the same reason the operations-as-data registry does (ADR-0209 bearing 1): every consumer surface must read it without a new dependency edge.

3. **One decision function, one chokepoint per surface.** `authorization::authorize` is the only place the rule is evaluated. It is reached from exactly two enforcement points, each of which its whole surface must traverse: `OrbitRuntime::run_tool_with_context_and_role` for every tool call (CLI `tool run`, the CLI admin bypass, MCP `tools/call`, the dashboard, the v2 deterministic dispatcher, agent loops), and the `Commands::operation` dispatch in `orbit-cli`'s `main` for CLI commands that destroy without a tool. Neither reimplements any part of the rule.

4. **The run's sanction travels with the run, not with the environment.** The distinction drawn is out-of-scope destruction versus destruction the run exists to perform. `run_deterministic` stamps `McpCapability::Runner` onto the tool context it dispatches with, so `release_locks` keeps working while the agent hosted inside the same run — which builds its own context and never reaches that code — does not inherit the grant. The pipeline's git path (worktree force-removal, `branch -D`, `git clean -fd`, `checkout -B`, `merge --ff-only`, PR merge) is deliberately absent from the registry: it is sanctioned destruction, and gating it would break every ship.

5. **Ambiguity fails closed, with one explicit escape hatch.** Caller capabilities resolve in strict precedence: session grants (a validated MCP session, or a run-stamped context) > `ORBIT_OPERATOR` override > agent envelope (`ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL` / `ORBIT_TASK_ACTOR_KIND=agent` / `ORBIT_MANAGED_RUN_CONTEXT`) > interactive terminal > nothing. An unidentified caller holds no capability and every governed operation denies. An interactive TTY on both stdin and stderr is the one positive human signal available without a credential, which is what keeps `orbit workspace teardown` frictionless for a person while refusing it to a headless agent. `ORBIT_OPERATOR=1` is the documented escape hatch — trivial to set on purpose, because it is not defending against a determined caller, it is making a deliberate act look deliberate. Every use emits a `warn!` and a success-status `authorization` audit row.

6. **A denial is a distinct, recorded outcome.** `OrbitError::CapabilityDenied` is separate from `OrbitError::PolicyDenied` (`orbit-policy`'s path-scoping refusal): this one is about who is asking, not which path was touched. Both map to `AuditEventStatus::Denied` at every audit boundary, and the decision writes its own `command = 'authorization'` row so a denial is queryable with one predicate across every entry point, including surfaces that do not otherwise audit. The message names the required capability, why the operation is governed, what the caller was resolved as, and the escape hatch.

7. **Remove the approval config rather than wire it.** `task.approval.required_for_agent` / `delegate_approval` and `OrbitError::TaskApprovalRequired` are deleted. `required_for_agent` expressed "an agent may not unilaterally approve", which the capability model now states directly and enforces; `delegate_approval` had no consumer and no coherent meaning without a delegation surface. Keeping a second, unread knob for the same intent would reintroduce exactly the advertisement-versus-enforcement gap this ADR closes.

### Rejected alternatives

- **Per-subcommand guards.** Rejected: `orbit tool run` and every future entry point reopen the hole. This is the failure mode the task was filed against.
- **A credential, password, or keychain gate.** Rejected on the framing above — no protection, and it would misrepresent the boundary.
- **Making `ToolRegistry::execute` consult `ToolAvailability`.** Rejected: availability is an MCP advertisement concept, and enforcing it there would refuse operator callers the CLI legitimately serves, deepening rather than resolving the advertisement/enforcement conflation.
- **Converting the `register_inactive` builtins to `operator_only` policies.** Attractive — advertisement would then derive from the same declaration — but it changes what operator MCP sessions can see, and the capability chokepoint closes the actual bypass without that blast radius. Filed as ORB-10478, which also owns the residual gap noted in Consequences.

### Consequences

- A single predicate (`command = 'authorization'`) answers "what was refused, to whom, and why" across CLI, MCP, dashboard, and engine. Denials are separable from failures in the audit trail for the first time.
- Adding a governed operation is one registry entry; the chokepoints need no edit. The capability required by an operation exists in exactly one enumerable place.
- Cost: a non-interactive caller with no session grant and no agent envelope — a shell script, a cron entry, a test binary — is now refused governed operations and must set `ORBIT_OPERATOR=1`. This is a real behavior change for automation that previously worked silently, and it landed as churn in three CLI integration tests. It is the intended trade: an unidentified caller performing destruction is precisely the accident being guarded against.
- Cost: the escape hatch is an environment variable an agent can set, so a *determined* agent is not stopped. That is by construction, not oversight — see the framing. The value delivered is the loud failure and the record, not prevention.
- Cost: the `governed_when` predicate for flag-gated commands (`--confirm`) lives in the `Commands::operation` arm, so a new destructive CLI command must say that it is one. The requirement itself still lives only in the registry, but the invocation-to-operation mapping is a second thing to remember. The compiler cannot catch a missing mapping.
- Residual gap, tracked as **ORB-10478**: tools declared `McpExposure::OperatorOnly` (`orbit.friction.list` / `show` / `update`) are refused by the hub MCP *call* path (`admitted()` in `crates/orbit-remote/src/mcp/hub.rs`) but are absent from the registry, so `orbit tool run` still performs them for any caller. They are left ungoverned deliberately: `OperatorOnly` currently encodes audience/placement rather than destructiveness, two of the three are non-destructive reads, and enforcing MCP `allowed_capabilities()` at the tool chokepoint would refuse agents ordinary `orbit friction list` reads — a regression, not a fix. Closing it means separating "who may see this" from "who may perform this", which is ORB-10478's job.
- The registry is intentionally small and opt-in. It names the operations whose accidental invocation destroys something, not every mutating tool; broadening it is a judgment call to be made deliberately, per operation.

## Task References

- [ORB-10358] — piloted ADR-0209 bearing 1 on the friction noun; produced the
  split table, the derived adapters, and the frozen-surface method.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

## Context

The code graph is currently exposed on two surfaces: the `orbit graph` CLI (`orbit-graph-cli`, structured JSON on stdout/stderr) and 10 `orbit.graph.*` MCP tools served by `orbit-remote/src/mcp/graph.rs` (~600 lines mirroring the CLI, plus schema/e1 tests) off a long-lived `GraphToolRegistry` that caches per-repo `Graph` handles with debounced sync. `orbit-common`'s tool allowlist bakes in the `orbit.graph.` wildcard root, and the allowlist comment notes the tools are served in-process for agent runs.

The consumer profile does not match the external MCP surface. Graph is code intel for agents sitting inside a repo checkout with shell access (in-session repo agents, worker leaf runs), all of which can invoke the CLI directly. Orchestrators do not do deep code navigation per the router boundary, and Bridge's MCP surface already omits graph.

Guiding lesson (2026-07-18): adopt a tool for the smallest footprint where it's the best option — a tool that's too broad overlaps existing pieces and creates incompatibility, and past a point extra capability doesn't justify the extra space it claims.

## Decision (amended 2026-07-19 — see Amendment)

Rescoped: remove `orbit.graph.*` from the **external/remote MCP surface only**. The **in-process graph tool serving for engine-internal shell-less roles is retained** — it is load-bearing (see Amendment). Agents with shell access use `orbit graph <cmd>`, which already emits structured JSON.

Do not build a separate graph MCP server now. `orbit-graph` is a clean crate boundary, so packaging it later stays cheap. The trigger for a standalone server would be an **external** consumer with MCP access but no shell; internal engine roles are served in-process.

## Amendment (2026-07-19)

The original decision assumed no shell-less consumer of the graph MCP tools existed. The ORB-10325 pre-flight audit falsified this: the planning-duel PLANNER and ARBITER roles (`orbit-engine/.../planning_duel/roles.rs`) are deliberately shell-less (`proc_allowed_programs` empty, no `proc.spawn`) and their instructions centrally require graph navigation (enumerate callers by name, bound blast radius, confirm import direction). `fs.read` cannot substitute, and granting them `proc.spawn` on the `orbit` binary would widen two read-only roles to the full orbit CLI including durable writes — a larger footprint violation than the one this ADR removes. Decision rescoped accordingly: external exposure removed, in-process serving retained.

Implementation must first quantify how much code is exclusive to the external exposure versus shared with the in-process path; if the external-only share is trivial, report that instead of making a cosmetic change.

## Consequences

- The external MCP surface stops advertising 10 graph tool schemas; code exclusive to that exposure and its tests are removed. In-process serving for engine roles is unchanged and covered by tests.
- Vestigial graph grants in shell-less activities that never invoke graph tools (`dispatch_agent.yaml`, `epic_orchestrator.yaml`) are dropped; `deterministic_reference.yaml`'s fixture is repointed. `tool_allowlist.rs` graph entries REMAIN valid for in-process roles.
- Cost: the split surface (in-process yes, external no) is a subtler contract than "CLI only" — docs must state it explicitly, and the shared `GraphToolRegistry` maintenance largely remains.
- Cost: any future external shell-less consumer still requires building the deferred separate MCP server.
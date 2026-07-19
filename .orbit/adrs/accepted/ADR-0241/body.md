## Context

The code graph is exposed via the `orbit graph` CLI (`orbit-graph-cli`, structured JSON on stdout/stderr) and 10 `orbit.graph.*` MCP tools served by `orbit-remote/src/mcp/graph.rs` (~588 LOC, `GraphToolRegistry`, 10 schemas) on the broker composition. Investigation during ORB-10325 established the true topology: there is ONE broker surface serving both engine roles and local editors identically; the hub (the only real remote surface) already registers graph `recognition_only` and Bridge omits graph entirely. No separable "external exposure" exists at the broker.

Guiding lesson (2026-07-18): adopt a tool for the smallest footprint where it's the best option — extra capability doesn't justify the extra space it claims.

## Decision (second amendment, 2026-07-19)

Full removal restored. Remove the `orbit.graph.*` MCP tools from the broker surface entirely; `orbit graph` CLI (structured JSON) becomes the sole graph surface.

The first amendment kept in-process serving because the planning-duel planner/arbiter roles are shell-less and their instructions mandate graph navigation. Daniel's ruling: that dependency is prompt-imposed, not intrinsic — planning duels do not need call-graph-verified precision at authoring time; blast-radius verification belongs to the implement/review activities, which have shell and can run `orbit graph` directly. Accordingly:

1. Rewrite `PLANNING_DUEL_INSTRUCTION` and `ARBITER_INSTRUCTION` (`orbit-engine/.../planning_duel/roles.rs`) to plan and adjudicate via `fs.read` and search-level navigation, dropping the mandates that require graph tools; remove the 8 graph tool grants from `planner_activity`/`arbiter_activity`.
2. Remove `graph.rs`, the broker `advertised(graph)` and hub `recognition_only(graph)` registrations, graph entries in `canonical_mcp_tool_definitions`/`safe_mcp_tool_names`, schema metadata, and the graph-exercising tests/snapshots.
3. Remove graph entries from `tool_allowlist.rs` (all in-process consumers are gone once duel roles and vestigial grants are cleaned); drop vestigial grants in `dispatch_agent.yaml`/`epic_orchestrator.yaml`; repoint `deterministic_reference.yaml`'s fixture.
4. Shell-holding activities (agent_implement, agent_review, step_failure_recovery) use the `orbit graph` CLI via `proc.spawn`, as they already can.

Do not build a separate graph MCP server. The trigger remains an external consumer with MCP access but no shell; none exists.

## Consequences

- The broker sheds 10 tool schemas from every connecting client's tools/list (engine roles and local editors alike), ~588 LOC of serving code, and the graph test/snapshot surface; the `GraphToolRegistry` cache/staleness class disappears.
- Planning-duel planner/arbiter lose symbol-graph navigation; their instructions are correspondingly rewritten to a plan-level standard of evidence. Risk accepted by Daniel: plan precision claims are verified downstream at implement/review, which retain full graph access via CLI.
- Cost: if duel plan quality measurably degrades without graph navigation, the remedy is granting those roles a narrowly scoped capability or revisiting this ADR — a deliberate future decision, not a silent re-add.
- Cost: any future shell-less consumer requires building the deferred separate MCP server.
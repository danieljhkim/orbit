# Lessons Learned While Building Orbit

**Status:** Draft
**Owner:** Daniel
**Last updated:** 2026-04-24

I am dedicating this place to record some of the lessons we learned along the way. These lessons may not apply to everyone or in every case, but they shaped some of the decisions we made.

---

## 1. Tool surface decides which fight your tool has to win

I had been reluctant to expose orbit tools via MCP due to rising concerns about their higher token usage compared to CLI counterparts. This notion led me to stubbornly push for CLI as the primary tool interface for agents.

This all changed during our benchmarking session on graph tools. The benchmarking experiment had three groups:
- `graph-only`: only graph tools
- `hybrid`: graph tools plus `Read`, `Grep`, `Glob`
- `no-graph`: only `Read`, `Grep`, `Glob`

In [[v1](../benchmarks/graph/v1/RESULTS.md)] and [[v2](../benchmarks/graph/v2/RESULTS.md)], Codex was exposed to graph tools via the `exec_command` tool (i.e., `exec_command orbit tool run orbit.graph.search`). Claude, on the other hand, was exposed to graph tools via MCP only. In both v1 and v2 trials, `hybrid` Codex never reached for the graph tools over 60 runs, not even once.

In [[v3](../benchmarks/graph/v3/RESULTS.md)], we changed things up and gave Codex MCP graph tools instead. Hybrid Codex invoked them in **23 of 30** runs. Claude had MCP all along, yet used graph tools **just once** across 60 hybrid runs (v1 and v3; Claude was not run in v2). Same task, same backend, just different access surface.

When a lesser-known tool like `orbit.graph` competes against better-known primitives such as `rg` in the same access surface (i.e., `exec_command ...`), it loses. `rg` has higher base-rate probability than `orbit tool run orbit.graph.*` for grep-style lookups, and that prior flows into whatever sampling strategy is on top. For Claude, the same fight happens one level up (i.e., `Read`, `Grep`, `Glob` vs. `orbit_graph_search`), which is why the graph tool was invoked only once despite Claude having had MCP access all along.

In short, v3 results suggest MCP tools win the matchup against a generic `exec_command`, but struggle when the agent already has a specialized peer that does something similar.

**Lesson**: the original concern about MCP's higher token usage is real, and for esoteric tools without any competitors a CLI-based interface may work just fine without the additional MCP token tax. But when the goal is to expand the agent's toolset with specialized tools for specialized jobs, better pick the easier fight.

---
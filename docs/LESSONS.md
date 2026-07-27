---
type: context
summary: Lessons Learned While Building Orbit
last_validated: 2026-07-27
---

# Lessons Learned While Building Orbit

**Status:** Draft
**Owner:** Daniel
**Last updated:** 2026-05-11

I am dedicating this place to record some of the lessons we learned along the way. These lessons may not apply to everyone or in every case, but they shaped some of the decisions we made.

---

## 1. Tool surface decides which fight your tool has to win

I had been reluctant to expose orbit tools via MCP due to rising concerns about their higher token usage compared to CLI counterparts. This notion led me to stubbornly push for CLI as the primary tool interface for agents.

This all changed during our benchmarking session on graph tools. The benchmarking experiment had three groups:
- `graph-only`: only graph tools
- `hybrid`: graph tools plus `Read`, `Grep`, `Glob`
- `no-graph`: only `Read`, `Grep`, `Glob`

Historical graph benchmark rounds v1 and v2 (removed with the graph subsystem
under ADR-0291 / ORB-10491) exposed Codex to graph tools through shell execution
and Claude through MCP. In both rounds, hybrid Codex never reached for the graph
tools over 60 runs.

Historical v3 exposed the same retired tools to Codex through MCP. Hybrid Codex
invoked them in **23 of 30** runs. Claude had MCP all along, yet used graph tools
just once across 60 hybrid runs. Same task, same backend, different access
surface.

The durable lesson is about discoverability, not the removed implementation:
when a lesser-known tool competes with familiar primitives such as `rg` in the
same access surface, the familiar primitive wins. Moving a tool to a dedicated
discovery surface can materially change utilization without changing the
backend.

In short, v3 results suggest MCP tools win the matchup against a generic `exec_command`, but struggle when the agent already has a specialized peer that does something similar.

**Lesson**: the original concern about MCP's higher token usage is real, and for esoteric tools without any competitors a CLI-based interface may work just fine without the additional MCP token tax. But when the goal is to expand the agent's toolset with specialized tools for specialized jobs, better pick the easier fight.

---

## 2. The May 2026 Artifact Loss Incident

On 2026-05-11, hundreds of task artifacts were wiped out due to our reckless workspace cleanup. These artifacts are now gone for good, and can never be recovered. The only way to prevent this from happening again is to implement a backup and recovery system for task artifacts ADR-0149.

This was catastrophic, but also gave us a chance to amend for the sins of our bad design decisions that have been plaguing us for a while now. [docs/design/task-artifacts/4_decisions](design/task-artifacts/4_decisions.md)

**Lesson**: Backup and recovery are not optional for long-lived artifacts.

----

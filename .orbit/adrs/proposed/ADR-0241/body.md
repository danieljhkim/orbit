## Context

The code graph is currently exposed on two surfaces: the `orbit graph` CLI (`orbit-graph-cli`, structured JSON on stdout/stderr) and 10 `orbit.graph.*` MCP tools served by `orbit-remote/src/mcp/graph.rs` (~600 lines mirroring the CLI, plus schema/e1 tests) off a long-lived `GraphToolRegistry` that caches per-repo `Graph` handles with debounced sync. `orbit-common`'s tool allowlist bakes in the `orbit.graph.` wildcard root, and the allowlist comment notes the tools are served in-process for agent runs.

The consumer profile does not match the MCP surface. Graph is code intel for agents sitting inside a repo checkout with shell access (in-session repo agents, worker leaf runs), all of which can invoke the CLI directly. Orchestrators do not do deep code navigation per the router boundary, and Bridge's MCP surface already omits graph — no current MCP consumer exists that could not shell out.

Guiding lesson (2026-07-18): adopt a tool for the smallest footprint where it's the best option — a tool that's too broad overlaps existing pieces and creates incompatibility, and past a point extra capability doesn't justify the extra space it claims. The MCP graph surface duplicates the CLI's capability while claiming schema space in every session's tool list.

## Decision

Make `orbit-graph` CLI-surface only. Remove the `orbit.graph.*` tools from the MCP surface (`orbit-remote/src/mcp/graph.rs` and the in-process serving path). Agents needing code intel run `orbit graph <cmd>`, which already emits structured JSON.

Do not build a separate graph MCP server now. Keep it as the documented fallback: `orbit-graph` is already a clean crate boundary (dep-boundary tests exist), so packaging it later is cheap. The concrete trigger to build it is a consumer with MCP access but no shell.

Removal is done in one pass covering the full blast radius: the served tools, `tool_allowlist.rs` graph entries in `orbit-common` (removing served tools while leaving allowlist names validates-but-strands entries), any activity/job specs referencing `orbit.graph.*`, and the dep-boundary and schema/e1 tests.

## Consequences

- The MCP surface sheds 10 tool schemas per session and ~600 lines of adapter code plus its tests; the `GraphToolRegistry` cache-staleness/concurrency class (long-lived per-repo handles, debounced sync) disappears entirely — the fresh-process CLI has no equivalent.
- One capability is genuinely lost: policy-scoped, graph-only grants without shell access. An executor policy of "graph tools yes, exec no" is no longer expressible; anything needing graph must also hold shell. Verify no current activity relies on this before landing.
- Cost: any future shell-less consumer (restricted executor, remote-only client) must wait for the deferred separate MCP server to be built and operated — another binary to version in `install.sh`, register per client, and keep alive.
- Allowlist validation and any specs referencing `orbit.graph.*` must be updated in the same change; a partial removal leaves stranded-but-valid allowlist entries.
# Glossary — Orbit MCP Bridge

Vocabulary specific to the singular-hub, placement-aware Orbit MCP feature.
Standard MCP, SSH, workspace, task, and graph terms are excluded unless narrowed.

| Term | Meaning |
|------|---------|
| Caller-host provenance | Stable identity of the machine running the client-facing broker, recorded separately from the hub process host. See [2_design.md §9](../2_design.md). |
| Composite route | A canonical tool that deliberately executes more than one placement, such as hub ID allocation plus owner finalize or role-aware search. See [2_design.md §4](../2_design.md). |
| Current knowledge | The owner checkout's current learning/ADR state; reachable only on that owner, including through the hub link when the hub is the owner. See [2_design.md §6](../2_design.md). |
| Hub link | The one trusted in-process/SSH-carried MCP route from the local broker to the coordination hub. See [2_design.md §5](../2_design.md). |
| Hub mode | The non-recursive MCP server mode that executes coordination-plane tools and never connects to an owner/spoke. See [2_design.md §2.2](../2_design.md). |
| Hub route | Tool execution on the singular coordination hub, locally short-circuited or carried over the hub link. See [2_design.md §4](../2_design.md). |
| Local-derived route | Tool execution against rebuildable state derived from the exact current checkout/worktree, such as graph or docs indexes. See [2_design.md §4](../2_design.md). |
| Local MCP broker | The `orbit mcp serve` process registered with the client; preserves local checkout/role and dispatches canonical tools by placement. See [2_design.md §2.1](../2_design.md). |
| MCP call ID | Broker-generated correlation ID propagated to hub/local audit and returned for uncertain transport outcomes. See [2_design.md §9](../2_design.md). |
| Owner route | Current knowledge execution on the declared owner; reachable only when the owner is local or is the hub, never by opening a spoke-to-spoke route. See [2_design.md §4.2](../2_design.md). |
| Placement class | Canonical tool metadata declaring `hub`, `owner`, `local-derived`, or `composite`; independent of capability. See [2_design.md §4.1](../2_design.md). |
| Replica knowledge read | Explicitly requested learning/ADR read from a pulled and reindexed Git replica, marked with owner/commit/index freshness rather than presented as current. See [2_design.md §7](../2_design.md). |
| Transport trust | Machine-local permission in `~/.orbit/mcp.toml` to reach the one stable hub `machine_id`, pinned out of band before registration, through a configured SSH alias. See [2_design.md §1, §5.1](../2_design.md). |

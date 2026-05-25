---
name: orbit-graph
description: Use when navigating and inspecting codebase via the knowledge graph instead of raw file reads.
---

# Orbit Graph

Use `orbit.graph.*` as your default way to navigate code. Start with the smallest tool that can answer the question.

## Tool Invocation

Graph tools are available via two surfaces; both accept identical JSON.

- **MCP** (plugin path): `orbit_graph_sync`, `orbit_graph_search`, `orbit_graph_show`, `orbit_graph_refs`, `orbit_graph_callees`, `orbit_graph_impact`, `orbit_graph_trace`. Call them directly when loaded.
- **CLI**: `orbit tool run orbit.graph.<action> --input '<json>'`.

Mapping rule: `orbit.graph.<action>` ↔ `orbit_graph_<action>`. See the `orbit` skill for the full reference. Do not prefer shell just because the examples below use CLI syntax.

## Default Workflow

1. **Search first** — Use `orbit.graph.search` when the prompt names a symbol, string, config key, or file. Add `kind`, `lang`, and `limit` filters when useful.
2. **Inspect the exact selector** — Use `orbit.graph.show` to confirm the definition, source text, span, and metadata for the match you found.
3. **Use one relationship tool only if needed**:
   - `orbit.graph.refs` for inbound usages and structural relations
   - `orbit.graph.callees` for outbound calls from a symbol
   - `orbit.graph.impact` for a bounded blast-radius traversal around a symbol
   - `orbit.graph.trace` for command-handler call trees
   - `orbit.graph.history` has been removed from the agent tool surface; for task-to-commit lookup use `git log --grep '[T<task-id>]'`
4. **Sync explicitly for scripted checks** — Use `orbit.graph.sync` before timing-sensitive or batch queries; normal read tools keep the index fresh for interactive use.

## Task IDs

Orbit graph task attribution was removed. When the prompt asks what a task touched, use git's local commit-message convention instead:

```bash
git log --grep '[T20260421-0528]' --oneline
```

Orbit `task_id` is local to the operator's workspace. For cross-engineer task references, prefer `external_refs`.

## Stop Rule

If `search + show`, `refs`, `callees`, `impact`, or `trace` already answers the question, stop.

Do not fan out into repeated `show` calls when one relationship query gives the needed answer.

## When `fs.read` Is Acceptable

- Graph returned `knowledge_unavailable`
- A selector returned `null` and you only fall back for that entry
- You need a source-pattern enumeration that graph search does not support
- You need a few extra lines around a symbol you already found with graph tools

## Minimal Commands

```bash
# Exact symbol lookup
orbit tool run orbit.graph.search --input '{"query":"hello","kind":"symbol","limit":10}'
orbit tool run orbit.graph.show --input '{"selector":"symbol:src/lib.rs#hello:function"}'

# Refs, callees, impact
orbit tool run orbit.graph.refs --input '{"symbol":"symbol:src/lib.rs#hello:function","confidence":"same_module"}'
orbit tool run orbit.graph.callees --input '{"symbol":"symbol:src/lib.rs#hello:function"}'
orbit tool run orbit.graph.impact --input '{"selector":"symbol:src/lib.rs#hello:function","depth":3}'

# Command trace and explicit sync
orbit tool run orbit.graph.trace --input '{"command_name":"task update","depth":4}'
orbit tool run orbit.graph.sync --input '{"full":true}'

```

## Selector Forms

- `dir:<path>`
- `file:<path>`
- `symbol:<path>#<name>:<kind>`
- `module:<qualified>`
- `command:<name>`

Common symbol kinds: `function`, `method`, `struct`, `trait`, `impl`, `field`, `module`.

## Avoid

- Skipping graph tools and going straight to `fs.read`
- Expecting `orbit.graph.search` to support arbitrary source regex enumeration
- Using `orbit.graph.refs` when you need outbound calls; use `orbit.graph.callees`
- Using `orbit.graph.callees` when you need inbound usages; use `orbit.graph.refs`
- Expecting `orbit.graph.history` or `orbit.graph.search` to answer task attribution questions; `orbit.graph.history` is not agent-callable, and local task-to-commit lookup belongs to `git log --grep '[T<task-id>]'`
- Reading full files after `show` already gave the needed context
- Falling back to `fs.read` globally when only some selectors failed

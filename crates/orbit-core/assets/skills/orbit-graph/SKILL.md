---
name: orbit-graph
description: Use when navigating and inspecting the codebase via the orbit-graph code index instead of raw file reads.
---

# Orbit Graph

Use `orbit_graph_*` as your default way to navigate code. Start with the smallest tool that can answer the question.

## Tool Invocation

The graph is served **in-process over MCP** by orbit-graph (v2). Call the tools directly from your toolbox:

- `orbit_graph_sync`, `orbit_graph_search`, `orbit_graph_show`, `orbit_graph_refs`, `orbit_graph_callees`, `orbit_graph_impact`, `orbit_graph_trace`, `orbit_graph_overview`, `orbit_graph_implementors`, `orbit_graph_deps`.

There is **no** `orbit tool run orbit.graph.*` path and **no** `orbit graph` subcommand — the graph is not in the CLI tool registry. For direct human/shell use, the standalone `orbit-graph-cli` binary exposes the same queries as subcommands (`orbit-graph-cli search …`, `orbit-graph-cli refs …`, etc.).

The graph handle auto-syncs on a file watcher, so reads are normally fresh without an explicit sync. Call `orbit_graph_sync` only to force a refresh; pass `{"full": true}` for a complete re-index (required for full `trace` coverage — incremental sync resolves far fewer command handlers).

## Default Workflow

1. **Search first** — `orbit_graph_search` when the prompt names a symbol, trait, function, type, string, or config key. Narrow with `kind` (`symbol`, `string`, or `config`), `lang`, and `limit`.
2. **Inspect the exact selector** — `orbit_graph_show` to confirm the definition, source, and lines of the match you found.
3. **Use one relationship tool only if needed**:
   - `orbit_graph_implementors` for trait/interface implementation questions
   - `orbit_graph_refs` for inbound usages, cross-file references, **and caller-chain questions** (refs returns the inbound call sites; there is no separate `callers` tool)
   - `orbit_graph_callees` for outbound calls *from* a symbol
   - `orbit_graph_impact` for a bounded blast-radius traversal before an edit
   - `orbit_graph_trace` for a command handler's call tree (by command name)
   - `orbit_graph_deps` for module/import edges out of a `file:` or `dir:` selector
4. **Orient only when scope is unclear** — `orbit_graph_overview` when the subtree is unfamiliar or the task is architectural. Broad scopes default to `summary`; pass `format: "full"` only when you need per-file symbol lists.

## Confidence Floors

`refs`, `impact`, and `trace` accept a `confidence` floor: `exact`, `import`, `same_module` (default), or `fuzzy`. Every cross-file reference carries one of these; raise the floor to drop weakly-resolved edges, lower it to `fuzzy` to recover trait-dispatch and dynamic calls that only resolve by name.

## Stop Rule

If `search + show`, or `search + implementors`, or a single `search` already answers the question, stop.

Do not also run `overview`, `refs`, `callees`, or `impact` unless they add information the task still requires.

If you are about to call `show` on each candidate to verify which one matches, stop and reconsider — that is the verification-loop anti-pattern. Use the appropriate relation tool (`refs`, `callees`, `implementors`, `impact`) instead.

## Task IDs

Orbit graph task attribution was removed. When the prompt asks what a task touched, use git's local commit-message convention instead:

```bash
git log --grep '[T20260421-0528]' --oneline
```

Orbit `task_id` is local to the operator's workspace. For cross-engineer task references, prefer `external_refs`.

## When `fs.read` Is Acceptable

- A graph call errors or a selector does not resolve to a node
- You need a non-code file (config, YAML, TOML, markdown) that the index does not cover, and `orbit_graph_search` with `kind: "config"` did not surface it
- You need a few extra lines around a symbol you already found with graph tools

## Minimal Commands

These are MCP tool calls (JSON argument maps). The standalone `orbit-graph-cli` accepts the same fields as flags.

```jsonc
// Exact symbol lookup
orbit_graph_search   {"query":"hello","kind":"symbol","limit":10}
orbit_graph_show     {"selector":"symbol:src/lib.rs#hello:function"}
orbit_graph_search   {"query":"AGENT_ENV","kind":"config"}

// Trait/interface implementations
orbit_graph_implementors {"selector":"symbol:src/lib.rs#Greeter:trait"}

// Usages, caller chains, outbound calls, blast radius
orbit_graph_refs     {"symbol":"symbol:src/lib.rs#hello:function"}
orbit_graph_refs     {"symbol":"symbol:src/lib.rs#hello:function","confidence":"fuzzy"}
orbit_graph_callees  {"symbol":"symbol:src/lib.rs#hello:function"}
orbit_graph_impact   {"selector":"symbol:src/lib.rs#hello:function","depth":2}

// Command handler call tree (run a full sync first for complete coverage)
orbit_graph_sync     {"full":true}
orbit_graph_trace    {"command_name":"orbit.task.add"}

// Module/import edges and high-level shape
orbit_graph_deps     {"selector":"dir:crates/orbit-engine/src"}
orbit_graph_overview {"scope":"dir:src/module"}
orbit_graph_overview {"scope":"dir:src/module","format":"full"}
```

## Selector Forms

- `dir:<path>`
- `file:<path>`
- `symbol:<path>#<name>:<kind>`

Common symbol kinds: `function`, `method`, `struct`, `trait`, `impl`, `field`, `module`.

## Avoid

- Skipping graph tools and going straight to `fs.read`
- Running `orbit_graph_overview` by default for exact symbol lookups
- Reaching for `orbit.graph.pack`, `orbit.graph.callers`, or `orbit.graph.history` — those v1 tools no longer exist. Use `show` (per selector) for context, `refs` for callers, and `git log --grep` for task attribution.
- Using `orbit_graph_refs` for trait-implementation questions instead of `orbit_graph_implementors`
- Using `orbit_graph_refs` for crate dependency questions instead of `orbit_graph_deps`
- Reading full files after `show` already gave the needed context
- Falling back to `fs.read` for a symbol the graph can resolve

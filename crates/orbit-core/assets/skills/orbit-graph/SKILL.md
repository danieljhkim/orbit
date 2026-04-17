---
name: orbit-graph
description: Use when navigating and inspecting codebase via the knowledge graph instead of raw file reads.
---

# Orbit Graph

## Rule: Graph First, File Read Last

**You MUST use `orbit.graph.*` tools as your primary means of codebase navigation.** Do not default to `fs.read` for exploration or context gathering. Graph tools return scoped, symbol-level context — only the relevant symbols, imports, and neighbors — instead of dumping entire files into your context window.

**When to use graph tools:**
- Understanding codebase structure → `orbit.graph.overview`
- Finding symbols, types, functions → `orbit.graph.search`
- Gathering context for a task → `orbit.graph.pack`
- Tracing dependencies and callers → `orbit.graph.refs`
- Inspecting a specific node → `orbit.graph.show`

**When `fs.read` is acceptable:**
- Graph returned `knowledge_unavailable` (graph not built — this is the only valid reason to skip graph tools entirely)
- Specific selectors came back as `unresolved_selectors` (fall back only for those entries)
- You need to read a non-code file (config, YAML, TOML, markdown)
- You already used `orbit.graph.pack` and need to see additional lines around a specific symbol

**Never do this:**
- Skip graph tools because `fs.read` feels simpler
- Read entire files when you only need one function or struct
- Fall back to `fs.read` globally when only some selectors failed

## Workflow: Start Every Task Here

1. **Orient** — `orbit.graph.overview` with the relevant prefix to understand scope, node counts, languages, and symbol distribution.
2. **Search** — `orbit.graph.search` to find specific symbols, types, or functions. Use `kind` filter to narrow results.
3. **Trace** — `orbit.graph.refs` to find who calls or uses a symbol before modifying it.
4. **Gather** — `orbit.graph.pack` with selectors built from task context files and search results. This is your context — do NOT also `fs.read` the same files.
5. **Inspect** — `orbit.graph.show` for detailed lineage, siblings, and children of a node.

## Command Reference

All graph tool calls go through `orbit tool run`. **If running from a worktree**, pass `--root <original .orbit dir>`.

```bash
# Pack context from selectors (dir, file, or symbol)
orbit tool run orbit.graph.pack --input '{"selectors": ["dir:src", "file:src/lib.rs", "symbol:src/lib.rs#hello:function"]}'

# Search nodes — omit query to browse all nodes
orbit tool run orbit.graph.search --input '{"query": "hello", "type": "symbol", "kind": "function", "limit": 10}'

# Browse all nodes (no query)
orbit tool run orbit.graph.search --input '{"limit": 30}'

# Overview — aggregate summary of the graph
orbit tool run orbit.graph.overview --input '{"prefix": "crates/orbit-knowledge/src"}'

# Find references — who uses this symbol?
orbit tool run orbit.graph.refs --input '{"selector": "symbol:src/lib.rs#hello:function"}'

# Show node with lineage, siblings, children, source
orbit tool run orbit.graph.show --input '{"selector": "symbol:src/lib.rs#hello:function"}'
```

## Selector Syntax

| Form | Format | Example |
|------|--------|---------|
| Dir | `dir:<path>` | `dir:src/module` |
| File | `file:<path>` | `file:src/lib.rs` |
| Symbol | `symbol:<path>#<name>:<kind>` | `symbol:src/lib.rs#hello:function` |

Symbol kinds: `function`, `method`, `struct`, `trait`, `impl`, `class`, `interface`, `field`, `module`.

## Context Gathering Protocol

1. Build selectors from `task.context_files`: files become `file:<path>`, named symbols become `symbol:<path>#<name>:<kind>`.
2. Call `orbit.graph.pack` with the selector list.
3. Handle the response:
   - **Success**: Use pack entries as context. Do NOT also `fs.read` the full files.
   - **`knowledge_unavailable`** (check `kind` field): Fall back to `fs.read`. Normal for repos without a built graph.
   - **`unresolved_selectors`**: Fall back to `fs.read` only for those entries. Do NOT fall back globally.
4. Dir pack entries include `children` (child file/dir selectors). File pack entries include `symbol_summary` (name/kind/selector for each symbol in the file).

## Tool Reference

| Tool | Required Params | Optional Params |
|------|-----------------|-----------------|
| `orbit.graph.pack` | `selectors` (array) | `knowledge_dir` |
| `orbit.graph.search` | *(none)* | `query`, `type`, `kind`, `prefix`, `limit`, `format` |
| `orbit.graph.overview` | *(none)* | `prefix`, `knowledge_dir` |
| `orbit.graph.refs` | `selector` | `limit`, `knowledge_dir` |
| `orbit.graph.show` | `selector` | `depth`, `siblings`, `children` |

## Search Output Formats

`orbit.graph.search` returns structured results by default:

```json
{
  "total": 5,
  "results": [
    { "selector": "symbol:src/lib.rs#hello:function", "name": "hello", "kind": "function", "file": "src/lib.rs" }
  ]
}
```

Pass `"format": "selectors"` for legacy flat array output.

## Common Mistakes

| Mistake | Correction |
|---------|------------|
| Skipping graph tools and going straight to `fs.read` | Start with `orbit.graph.overview`, then search/pack |
| `orbit graph show ...` | Use `orbit tool run orbit.graph.show --input '{...}'` |
| Falling back to `fs.read` globally when some selectors resolved | Only fall back for `unresolved_selectors` entries |
| Treating `knowledge_unavailable` as fatal | Normal when graph not built; fall back to `fs.read` |
| Reading full files after successful pack | Pack entries already contain relevant source |
| Using `fs.read` to understand codebase structure | Use `orbit.graph.overview` and `orbit.graph.search` instead |

---
name: orbit-graph
description: Use when navigating and inspecting codebase via the knowledge graph instead of raw file reads.
---

# Orbit Graph

Use `orbit.graph.*` as your default way to navigate code. Start with the smallest tool that can answer the question.

## Default Workflow

1. **Search first** — Use `orbit.graph.search` when the prompt names a symbol, trait, function, type, or file. Add `type`, `kind`, and `prefix` filters when you can.
2. **Inspect the exact selector** — Use `orbit.graph.show` to confirm the definition, source, lines, or lineage of the match you found.
3. **Use one relationship tool only if needed**:
   - `orbit.graph.implementors` for trait or interface implementation questions
   - `orbit.graph.refs` for callers, usages, or dependency tracing
4. **Gather only when needed** — Use `orbit.graph.pack` only for a small set of exact selectors when you need multi-symbol context for synthesis, editing, or review.
5. **Orient only when scope is unclear** — Use `orbit.graph.overview` when the subtree is unfamiliar or the task is architectural.

## Stop Rule

If `search + show`, or `search + implementors`, already answers the question, stop.

Do not also run `overview`, `refs`, or `pack` unless they add information the task still requires.

## When `fs.read` Is Acceptable

- Graph returned `knowledge_unavailable`
- Some selectors were `unresolved_selectors` and you only fall back for those entries
- You need a non-code file such as config, YAML, TOML, or markdown
- You need a few extra lines around a symbol you already found with graph tools

## Minimal Commands

```bash
# Exact symbol lookup
orbit tool run orbit.graph.search --input '{"query":"hello","type":"symbol","kind":"function","limit":10}'
orbit tool run orbit.graph.show --input '{"selector":"symbol:src/lib.rs#hello:function"}'

# Trait/interface implementations
orbit tool run orbit.graph.implementors --input '{"trait_selector":"symbol:src/lib.rs#Greeter:trait"}'

# Callers / usages / dependency tracing
orbit tool run orbit.graph.refs --input '{"selector":"symbol:src/lib.rs#hello:function"}'

# Multi-symbol context
orbit tool run orbit.graph.pack --input '{"selectors":["file:src/lib.rs","symbol:src/lib.rs#hello:function"]}'

# High-level subtree shape
orbit tool run orbit.graph.overview --input '{"prefix":"src/module"}'
```

## Selector Forms

- `dir:<path>`
- `file:<path>`
- `symbol:<path>#<name>:<kind>`

Common symbol kinds: `function`, `method`, `struct`, `trait`, `impl`, `field`, `module`.

## Avoid

- Skipping graph tools and going straight to `fs.read`
- Running `orbit.graph.overview` by default for exact symbol lookups
- Using `orbit.graph.refs` for trait-implementation questions instead of `orbit.graph.implementors`
- Packing broad directories or many selectors just to explore
- Reading full files after `show` or `pack` already gave the needed context
- Falling back to `fs.read` globally when only some selectors failed

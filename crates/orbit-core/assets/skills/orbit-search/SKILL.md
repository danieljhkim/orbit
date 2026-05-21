---
name: orbit-search
description: Use when searching tasks, docs, learnings, or ADRs through the unified `orbit search` query surface; also covers the secondary `orbit semantic` lifecycle namespace for installing, removing, inspecting, and indexing the local embedding companion.
---

# Orbit Search

Use `orbit search` when you need to find project context by topic, literal phrase, or related task ID. The query surface is `orbit search`; the lifecycle surface is `orbit semantic install|uninstall|stats|index`.

`orbit graph` remains the tool for code-structure questions such as callers, refs, implementors, and symbol selectors. Search is for corpus retrieval; graph is for structural traversal.

## Query Surface

Both CLI and MCP expose the same operation.

| Tool | MCP | CLI |
|------|-----|-----|
| `orbit.search` | `orbit_search({...})` | `orbit tool run orbit.search --input '{...}'` |

Mapping rule: `orbit.search` ↔ `orbit_search`. See the `orbit` skill for the full surface mapping. Include `model` in JSON inputs when available for provenance; pass your agent family (`codex`, `claude`, `gemini`, or `grok`).

Use these shapes:

```bash
# Lexical global search across tasks, docs, learnings, and ADRs
orbit search "slow inference after model swap" --limit 5
orbit tool run orbit.search --input '{"query":"slow inference after model swap","limit":5,"model":"<agent-family>"}'

# Cross-artifact label and path filters
orbit search "scheduler" --tag perf --kind all
orbit search path crates/orbit-search/src/lib.rs --kind all
orbit tool run orbit.search --input '{"tag":"perf","kind":"all","model":"<agent-family>"}'
orbit tool run orbit.search --input '{"path":"crates/orbit-search/src/lib.rs","kind":"all","model":"<agent-family>"}'

# Hybrid BM25 + cosine over task fields; other kinds remain lexical
orbit search "agent loop deadlock" --hybrid --kind task --limit 5
orbit tool run orbit.search --input '{"query":"agent loop deadlock","hybrid":true,"kind":"task","limit":5,"model":"<agent-family>"}'

# Cosine neighbors of a known task
orbit search similar "<task-id>" --limit 5
orbit tool run orbit.search --input '{"semantic":"<task-id>","limit":5,"model":"<agent-family>"}'
```

`orbit search similar <id>` uses task vectors, so it requires the companion. The MCP tool keeps the parameterized form: `semantic: "<task-id>"`.

`--tag` uses AND semantics when repeated. `orbit search path <path>` is the CLI applicability lookup: task results use selector containment over `context_files`; learning and ADR results use glob-containment over their stored path scopes; docs are content-indexed and do not match by applicability path. The MCP equivalent is `{"path":"<path>"}`.

`--status` values must use `kind:value` tokens, for example `--status task:open,doc:active,adr:proposed`. Bare status tokens are rejected so same-named statuses across corpora stay unambiguous.

## Index Coverage

Lexical search covers tasks, docs, learnings, and ADRs. Vector search currently covers task fields only. When `--hybrid` is set for `--kind doc`, `--kind learning`, `--kind adr`, or those portions of `--kind all`, Orbit still uses lexical matching for those kinds.

The help text for `orbit search --help` carries this asymmetry because it is user-visible behavior, not an implementation accident.

## When To Use

Three patterns cover almost every call:

1. **Pre-create context check.** Before adding a new task, query the proposed title or description to find related prior work. Use `--hybrid --kind task` when vocabulary mismatch is likely; use plain lexical search when looking for exact names, errors, or paths.
2. **Pre-execute related lookup.** After `orbit.task.show` loads a task, call `orbit.search` with `semantic` set to that task ID to surface prior tasks the original author may not have linked in `context_files`. Skim the top 3-5; ignore irrelevant hits.
3. **Ad-hoc project search.** "Where did we decide X?" or "didn't we have a doc about Y?" starts with `orbit search "X" --kind all`.

## Stop Rule

If one well-formed `orbit search` query returns useful hits, stop and inspect them. Do not chain repeated rewrites to chase higher scores. If the top hits are unrelated, treat the work as new and move on.

Do not use search for "find every symbol matching pattern X"; use `orbit.graph.search` instead.

## Lifecycle Namespace

`orbit semantic` manages the local embedding companion. It is not the query namespace.

| Purpose | CLI | MCP |
|---------|-----|-----|
| Install companion/model | `orbit semantic install [--model MODEL] [--force]` | `orbit.semantic.install` |
| Remove companion/model | `orbit semantic uninstall [--model MODEL] [--all]` | `orbit.semantic.uninstall` |
| Show status | `orbit semantic stats` | `orbit.semantic.stats` |
| Rebuild task embeddings | `orbit semantic index [--model MODEL] [--force]` | `orbit.semantic.index` |

Do not run `install` without operator consent. If a semantic query fails because the companion is missing, fall back to plain `orbit search --kind task <query>` (lexical) and continue unless the user explicitly asked to enable embeddings.

## Result Shape

`orbit.search` returns `mode`, `kind`, `notes`, and `results`. Each result has `kind`, `source`, and one or more of `id`, `path`, `title`, `summary`, `status`, `best_field`, `snippet`, `score`, and `matched_by`.

Treat scores as relative ordering signals, not absolute confidence thresholds. Snippets and matched fields are the most useful parts to read before deciding whether a hit is relevant.

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|--------------|--------------|
| Calling lifecycle commands to search | `orbit semantic` manages the companion | Use `orbit search` or `orbit.search` |
| Aborting when the companion is not installed | Embeddings are optional infrastructure | Fall back to lexical search unless the user opted in |
| Using semantic search for exact identifiers | Lexical matching is cheaper and more predictable for names, paths, and error strings | Use plain `orbit search` or `orbit.graph.search` |
| Calling `--semantic <id>` on a brand-new task before indexing completes | New tasks may not have embeddings yet | Search the task title/description with `--hybrid --kind task` |

## Cross-References

- `orbit-graph` — code-structure queries.
- `orbit-create-task` — optional pre-create search.
- `orbit-execute-task` — pre-execute related-task lookup.

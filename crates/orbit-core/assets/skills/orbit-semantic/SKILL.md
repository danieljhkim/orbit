---
name: orbit-semantic
description: Use when you need to find tasks by topic rather than literal substring — related-task lookup for prior context, "didn't we have a task about X?" questions, or any time you suspect prior work exists under different vocabulary than your query. Covers `orbit.semantic.search` (hybrid BM25 + cosine over indexed task fields) and `orbit.semantic.related` (cosine neighbors of a known task).
---

# Orbit Semantic

Use `orbit.semantic.*` when literal substring matching is the wrong lens — when the asker's vocabulary won't match the document's, or when the goal is *related* tasks rather than *matching* ones. The graph (`orbit.graph.*`) is for code structure; semantic is for task content.

## Companion Precondition

The semantic backend lives in a separately-installed `orbit-embed-companion` binary. **All `orbit.semantic.*` calls fail with a clear error pointing at `orbit semantic install` if the companion is missing.** If you hit that error and the operator hasn't opted into semantic search, fall back to `orbit.task.search` (lexical) and continue — do not block on the missing companion.

## Tool Invocation

Both tools are available via two surfaces; both accept identical JSON.

| Tool | MCP | CLI |
|------|-----|-----|
| `orbit.semantic.search` | `orbit_semantic_search({...})` | `orbit tool run orbit.semantic.search --input '{...}'` |
| `orbit.semantic.related` | `orbit_semantic_related({...})` | `orbit tool run orbit.semantic.related --input '{...}'` |

Mapping rule: `orbit.semantic.<action>` ↔ `orbit_semantic_<action>`. See the `orbit` skill for the full surface mapping. **Always include `model` in the JSON** for provenance; pass your agent family (`codex`, `claude`, `gemini`, or `grok`).

## When To Use

Three patterns cover almost every legitimate call:

1. **Pre-create context check (optional).** Before adding a new task, you can query the proposed title/description to find related prior work. Useful when the proposed task might overlap with an existing one that used different vocabulary. Not required — agents are free to skip if the work is clearly new.
2. **Pre-execute related lookup.** After `orbit.task.show` loads a task, call `orbit.semantic.related` on its ID to surface prior tasks the original author may not have linked via `context_files`. Skim the top 3–5 — usually one is genuinely useful, the rest are noise. See `orbit-execute-task` Step 1.
3. **Ad-hoc topic search.** "Have we worked on X before?" — call `orbit.semantic.search` directly; no task ID needed.

For "find every symbol matching pattern X" questions, use `orbit.graph.search` with `source_regex` instead — that's a structural query, not a semantic one.

## Stop Rule

If `orbit.semantic.search` with a well-formed query already returned a usable result, stop. Do not re-run with reworded queries to chase higher scores — RRF fusion already balances literal and semantic match. If the top hits are all unrelated, the work is genuinely new; proceed.

Do not chain `semantic.search` → `semantic.related` → `semantic.search` to widen the net. One pass plus `orbit.task.show` on promising hits is the standard depth.

## Minimal Commands

```bash
# Topic search across all indexed task fields (hybrid BM25 + cosine)
orbit tool run orbit.semantic.search --input '{"query":"slow inference after nomic swap","limit":5,"model":"<agent-family>"}'

# Restrict to a specific field (title, description, plan, acceptance, execution_summary)
orbit tool run orbit.semantic.search --input '{"query":"agent loop deadlock","field":"plan","limit":5,"model":"<agent-family>"}'

# Neighbors of a known task — useful before starting execution
orbit tool run orbit.semantic.related --input '{"id":"T20260510-3","limit":5,"model":"<agent-family>"}'
```

## Result Shape

Both tools return `results: [{source_id, source_kind, best_field, score, score_breakdown, snippet}]`. Treat `score` as a relative ordering signal, not an absolute calibration — there is no universal threshold. The `snippet` is the single most informative output: read it before deciding whether a hit is real.

## Index Freshness

Indexing happens automatically on task mutation; the worker is in `crates/orbit-embed/src/vector/worker.rs`. If `semantic.search` returns stale results after recent task edits, the operator can force a rebuild via `orbit semantic reindex`. As an agent, do not call `reindex` reflexively — only escalate if results are demonstrably stale during the same session.

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|--------------|--------------|
| Calling `orbit.semantic.*` and aborting when the companion isn't installed | Hard-failing a workflow on optional infrastructure | Catch the install-pointer error and continue with `orbit.task.search` |
| Re-running search with reworded queries to chase score | RRF already balances; rewording rarely moves the top result meaningfully | Inspect snippets, not scores; if no hit looks relevant, treat the work as new |
| Using semantic for literal identifier lookup (function name, error code, task ID) | BM25 inside the hybrid already handles those better than embeddings | Use `orbit.task.search` or `orbit.graph.search` for literal-identifier queries |
| Using `orbit.semantic.related` on a brand-new task before it's been indexed | Just-created tasks may not yet have embeddings ready | Use `orbit.semantic.search` on the task's title/description instead |

## Cross-References

- `orbit-graph` — complementary tool surface for *code-structure* queries (callers, refs, impls, file/symbol selectors). Graph and semantic do not overlap; both are useful together.
- `orbit-create-task` Step 2 — semantic search is available as optional context-gathering during task authoring.
- `orbit-execute-task` Step 1 — recommends the pre-execute related-task lookup.

# Graph Latency v2 Method

kind: perf-method
status: living
task: ORB-00380
date: 2026-06-13

## Question

Measure the read-path effect of serving `orbit.graph.show`, `orbit.graph.refs`, and
`orbit.graph.callers` from the orbit-knowledge SQLite index instead of hydrating
the full object graph on every call.

## Corpus

The first v2 measurement uses the Orbit repository itself at the ORB-00380
worktree revision. The generated knowledge graph contains:

- `node`: 18,640 rows
- `dir` / `file` / `leaf`: 436 / 1,740 / 16,464
- `source_mention`: 1,109,418 rows
- `call_edge`: 51,529 rows

This matches the large-repo shape that motivated ORB-00380 closely enough to
compare O(result) indexed reads against the legacy O(repo) fallback.

## Procedure

1. Build `target/debug/orbit`.
2. Warm the knowledge graph with `orbit.graph.search`.
3. Pin reads to the current branch ref so the measurement excludes refresh work:
   `ref = "orbit/ORB-00380-6a2daa9c"`.
4. Run 10 invocations per tool through `target/debug/orbit tool run`.
5. Measure wall-clock time around the process invocation with Python
   `time.perf_counter()`.
6. For the legacy comparison, set `ORBIT_GRAPH_DEBUG_FORCE_FALLBACK=1`, which
   makes `GraphIndexReader::open` decline the SQLite path and forces the
   read_graph implementations.

The selector used for all three tools:

```text
symbol:crates/orbit-knowledge/src/graph/sqlite_index/writer.rs#write_graph_index:function
```

The reported numbers include process startup and JSON serialization. They are
therefore conservative for in-process MCP use, but the before/after comparison
uses the same harness.

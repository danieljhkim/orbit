# Graph Token-Usage Benchmark - v4 Codex Results

**Status:** Codex provider complete. Claude is intentionally deferred until `T20260425-0739` is patched and the graph index is rebuilt.
**Sweep dates:** 2026-04-24 to 2026-04-25
**Sweep IDs:** `20260424-230632-f36f84` (`no-graph`, `graph-only`), `20260425-001959-842b8c` (`hybrid`)
**Sweep seeds:** `928176111` (`no-graph`, `graph-only`), `583229300` (`hybrid`)
**Harness SHA at report write:** `b0ce189e7053409c8754865bd154cd20e1de66a6`
**Scope:** Codex (`gpt-5.3-codex`) only: `no-graph` 12 fixtures x 3 seeds, `graph-only` 12 fixtures x 3 seeds, `hybrid` 8 fixtures x 3 seeds = 96 cells. No errored cells.

---

## Headline

1. **Codex no-graph stayed perfect:** 36/36 pass, median 11,446 total tokens.
2. **Codex graph-only was mixed:** 30/36 pass, median 15,462 total tokens. All six failures came from two production fixtures: `module-surface-orbit-mcp` and `reverse-export-orbit-error`.
3. **Codex hybrid was the strongest operating mode:** 24/24 pass on the hybrid-eligible subset, median 3,900 total tokens, with graph used in 11/24 runs.
4. **Graph has clear wins when the tool matches the question:** `deps-downstream-orbit-knowledge` (0.14x no-graph tokens), `callers-2hop-graphbenchpolicy` (0.44x), `impl-divergence-trait-method` (0.48x), `macro-expanded-callers` (0.59x), and `implementors-benchsink-with-blanket` (0.64x).
5. **The main graph-only losses expose a known graph bug, not just model confusion:** `T20260425-0739` tracks that `pub use` re-exports are dropped from graph file `exports` metadata. That bug directly explains `reverse-export-orbit-error` and likely contributes to `module-surface-orbit-mcp`.
6. **Schema ergonomics remain costly:** 28 graph tool calls failed and were mostly recovered. The dominant shape was scalar string input for string-list parameters (`refs.include`, `pack.selectors`).

**Known active bug:** `T20260425-0739` ("Graph parser misses pub-use re-exports in file exports metadata") means this Codex result is a pre-fix baseline for re-export/root-surface fixtures. Do not run or interpret the Claude v4 sweep until that bug is fixed and the graph index is regenerated.

---

## Arm Summary

| arm | runs | pass | median_total_tokens | p90_total_tokens | graph_call_rate | graph_calls | failed_graph_calls | shell_or_fs_calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| no-graph | 36 | 36/36 | 11446 | 27792 | 0/36 | 0 | 0 | 197 |
| graph-only | 36 | 30/36 | 15462 | 64877 | 36/36 | 334 | 25 | 0 |
| hybrid | 24 | 24/24 | 3900 | 11048 | 11/24 | 40 | 3 | 61 |

Hybrid only ran on the 8 graph-strength and precision-gap fixtures, per `METHOD.md`.
On that same 24-run subset, medians were: no-graph 11,446, graph-only 15,114, hybrid 3,900.

---

## Primary Aggregate

Verbatim from:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v4/runs \
  --tasks benchmarks/graph/v4/tasks
```

| provider | arm | task_class | runs | pass_rate | median_total_tokens | p90_total_tokens | tokens_per_success | graph_calls | graph_call_rate | shell_or_fs_calls |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| codex | graph-only | graph-strength | 12 | 75% | 9352 | 78425 | 33907 | 115 | 12/12 = 100.0% | 0 |
| codex | graph-only | payload-volume | 6 | 100% | 16855 | 101587 | 29067 | 57 | 6/6 = 100.0% | 0 |
| codex | graph-only | precision-gap | 12 | 100% | 15450 | 24307 | 14631 | 91 | 12/12 = 100.0% | 0 |
| codex | graph-only | selector-ambiguity | 6 | 50% | 21961 | 37822 | 44688 | 71 | 6/6 = 100.0% | 0 |
| codex | hybrid | graph-strength | 12 | 100% | 4232 | 14417 | 5394 | 26 | 9/12 = 75.0% | 23 |
| codex | hybrid | precision-gap | 12 | 100% | 3346 | 7607 | 3871 | 14 | 2/12 = 16.7% | 38 |
| codex | no-graph | graph-strength | 12 | 100% | 13154 | 26230 | 14676 | 0 | 0/12 = 0.0% | 67 |
| codex | no-graph | payload-volume | 6 | 100% | 12434 | 24527 | 13374 | 0 | 0/6 = 0.0% | 36 |
| codex | no-graph | precision-gap | 12 | 100% | 11169 | 15969 | 8733 | 0 | 0/12 = 0.0% | 42 |
| codex | no-graph | selector-ambiguity | 6 | 100% | 19298 | 51473 | 21861 | 0 | 0/6 = 0.0% | 52 |

---

## Category Aggregate

`median go/ng` compares aggregate category medians. `mean fixture go/ng` and `worst fixture go/ng` are per-fixture ratios and are the load-bearing cost readings.

| category | no-graph pass | graph-only pass | pass delta | median go/ng | mean fixture go/ng | worst fixture go/ng | hybrid graph rate |
|---|---:|---:|---:|---:|---:|---:|---:|
| graph-strength | 12/12 | 9/12 | -3 | 0.71x | 1.70x | 5.59x | 9/12 |
| precision-gap | 12/12 | 12/12 | +0 | 1.38x | 2.15x | 5.41x | 2/12 |
| payload-volume | 6/6 | 6/6 | +0 | 1.36x | 1.87x | 3.25x | n/a |
| selector-ambiguity | 6/6 | 3/6 | -3 | 1.14x | 1.45x | 1.89x | n/a |

---

## Production vs Synthetic

| mode | arm | runs | pass | median_total_tokens | graph_call_rate | graph_calls | failed_graph_calls |
|---|---|---:|---:|---:|---:|---:|---:|
| production | no-graph | 21 | 21/21 | 14162 | 0/21 | 0 | 0 |
| production | graph-only | 21 | 15/21 | 17847 | 21/21 | 222 | 18 |
| production | hybrid | 9 | 9/9 | 4872 | 3/9 | 3 | 0 |
| synthetic | no-graph | 15 | 15/15 | 11278 | 0/15 | 0 | 0 |
| synthetic | graph-only | 15 | 15/15 | 14972 | 15/15 | 112 | 7 |
| synthetic | hybrid | 15 | 15/15 | 3819 | 8/15 | 37 | 3 |

The production split is the load-bearing product signal. It is also where graph-only lost accuracy: both failing fixtures are production-grounded.

---

## Per-Fixture Table

| fixture | class | mode | arm | pass | median_tokens | p90_tokens | graph_call_rate | graph_calls | failed_graph_calls | shell/fs_calls |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | no-graph | 3/3 | 12397 | 23855 | 0/3 | 0 | 0 | 10 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | graph-only | 3/3 | 5444 | 15280 | 3/3 | 16 | 1 | 0 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | hybrid | 3/3 | 5495 | 5891 | 3/3 | 14 | 0 | 1 |
| `const-value-extraction` | payload-volume | production | no-graph | 3/3 | 8680 | 9445 | 0/3 | 0 | 0 | 21 |
| `const-value-extraction` | payload-volume | production | graph-only | 3/3 | 28222 | 101587 | 3/3 | 33 | 2 | 0 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | no-graph | 3/3 | 2818 | 12423 | 0/3 | 0 | 0 | 8 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | graph-only | 3/3 | 15257 | 17551 | 3/3 | 20 | 1 | 0 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | hybrid | 3/3 | 2873 | 4957 | 0/3 | 0 | 0 | 6 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | no-graph | 3/3 | 16379 | 25955 | 0/3 | 0 | 0 | 30 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | graph-only | 3/3 | 2250 | 11335 | 3/3 | 6 | 0 | 0 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | hybrid | 3/3 | 1383 | 1449 | 3/3 | 3 | 0 | 0 |
| `function-as-value-vs-direct-call` | precision-gap | production | no-graph | 3/3 | 14162 | 16744 | 0/3 | 0 | 0 | 23 |
| `function-as-value-vs-direct-call` | precision-gap | production | graph-only | 3/3 | 16761 | 23051 | 3/3 | 27 | 3 | 0 |
| `function-as-value-vs-direct-call` | precision-gap | production | hybrid | 3/3 | 4874 | 8743 | 0/3 | 0 | 0 | 21 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | no-graph | 3/3 | 11132 | 11207 | 0/3 | 0 | 0 | 4 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | graph-only | 3/3 | 15741 | 24846 | 3/3 | 22 | 1 | 0 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | hybrid | 3/3 | 3819 | 4741 | 2/3 | 14 | 1 | 4 |
| `impl-divergence-trait-method` | payload-volume | production | no-graph | 3/3 | 16776 | 24527 | 0/3 | 0 | 0 | 15 |
| `impl-divergence-trait-method` | payload-volume | production | graph-only | 3/3 | 8081 | 17847 | 3/3 | 24 | 1 | 0 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | no-graph | 3/3 | 11469 | 22553 | 0/3 | 0 | 0 | 7 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | graph-only | 3/3 | 7370 | 34916 | 3/3 | 32 | 2 | 0 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | hybrid | 3/3 | 3981 | 14873 | 3/3 | 9 | 2 | 7 |
| `macro-expanded-callers` | precision-gap | synthetic | no-graph | 3/3 | 11278 | 11512 | 0/3 | 0 | 0 | 7 |
| `macro-expanded-callers` | precision-gap | synthetic | graph-only | 3/3 | 6605 | 14098 | 3/3 | 22 | 2 | 0 |
| `macro-expanded-callers` | precision-gap | synthetic | hybrid | 3/3 | 2288 | 2485 | 0/3 | 0 | 0 | 7 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | no-graph | 3/3 | 5886 | 7437 | 0/3 | 0 | 0 | 16 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | graph-only | 0/3 | 11134 | 19582 | 3/3 | 42 | 5 | 0 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | no-graph | 3/3 | 32141 | 51473 | 0/3 | 0 | 0 | 36 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | graph-only | 3/3 | 32146 | 37822 | 3/3 | 29 | 4 | 0 |
| `reverse-export-orbit-error` | graph-strength | production | no-graph | 3/3 | 13912 | 26349 | 0/3 | 0 | 0 | 20 |
| `reverse-export-orbit-error` | graph-strength | production | graph-only | 0/3 | 77792 | 78697 | 3/3 | 61 | 3 | 0 |
| `reverse-export-orbit-error` | graph-strength | production | hybrid | 3/3 | 5830 | 13354 | 0/3 | 0 | 0 | 15 |

---

## Hybrid Utilization

| fixture | pass | median_tokens | graph_call_rate | graph_calls | shell/fs_calls | interpretation |
|---|---:|---:|---:|---:|---:|---|
| `callers-2hop-graphbenchpolicy` | 3/3 | 5495 | 3/3 | 14 | 1 | Used graph consistently; stayed well below no-graph. |
| `construct-vs-match-benchevent-distinct` | 3/3 | 2873 | 0/3 | 0 | 6 | Passed by shell fallback; graph avoided organically. |
| `deps-downstream-orbit-knowledge` | 3/3 | 1383 | 3/3 | 3 | 0 | Best graph-shaped win; deps solved directly. |
| `function-as-value-vs-direct-call` | 3/3 | 4874 | 0/3 | 0 | 21 | Passed by shell fallback; graph avoided organically. |
| `generic-dispatch-concrete-impl` | 3/3 | 3819 | 2/3 | 14 | 4 | Mixed graph use; source reading did the final disambiguation. |
| `implementors-benchsink-with-blanket` | 3/3 | 3981 | 3/3 | 9 | 7 | Used graph, then shell/source checks; cheaper than both baselines. |
| `macro-expanded-callers` | 3/3 | 2288 | 0/3 | 0 | 7 | Passed by shell fallback; graph avoided organically. |
| `reverse-export-orbit-error` | 3/3 | 5830 | 0/3 | 0 | 15 | Passed by shell fallback; graph-only failed all seeds. |

Hybrid's 24/24 pass rate is not proof that every hybrid-eligible fixture is graph-shaped. It is proof that Codex can route around graph gaps when shell/source tools are available.

---

## Graph-Only Cost Ratios

| fixture | class | mode | no-graph median | graph-only median | go/ng | graph-only pass |
|---|---|---|---:|---:|---:|---:|
| `deps-downstream-orbit-knowledge` | graph-strength | production | 16379 | 2250 | 0.14x | 3/3 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | 12397 | 5444 | 0.44x | 3/3 |
| `impl-divergence-trait-method` | payload-volume | production | 16776 | 8081 | 0.48x | 3/3 |
| `macro-expanded-callers` | precision-gap | synthetic | 11278 | 6605 | 0.59x | 3/3 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | 11469 | 7370 | 0.64x | 3/3 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | 32141 | 32146 | 1.00x | 3/3 |
| `function-as-value-vs-direct-call` | precision-gap | production | 14162 | 16761 | 1.18x | 3/3 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | 11132 | 15741 | 1.41x | 3/3 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | 5886 | 11134 | 1.89x | 0/3 |
| `const-value-extraction` | payload-volume | production | 8680 | 28222 | 3.25x | 3/3 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | 2818 | 15257 | 5.41x | 3/3 |
| `reverse-export-orbit-error` | graph-strength | production | 13912 | 77792 | 5.59x | 0/3 |

---

## Tool Diagnostics

Per-tool response size is measured in response characters, not model tokens. The transcripts do not expose per-tool token attribution.

| tool | invocations | succeeded | failed | success_rate | median_response_chars | p90_response_chars |
|---|---:|---:|---:|---:|---:|---:|
| callers | 16 | 16 | 0 | 100% | 1838 | 382947 |
| refs | 61 | 39 | 22 | 64% | 494 | 90616 |
| implementors | 13 | 12 | 1 | 92% | 1122 | 1249 |
| deps | 6 | 6 | 0 | 100% | 606 | 606 |
| pack | 69 | 65 | 4 | 94% | 1724 | 19595 |
| search | 103 | 102 | 1 | 99% | 260 | 2653 |
| show | 84 | 84 | 0 | 100% | 777 | 1587 |
| overview | 22 | 22 | 0 | 100% | 2436 | 59717 |

Failed graph calls by message:

| message | count | affected tools |
|---|---:|---|
| `invalid input: include must be an array of strings` | 22 | `refs` |
| `invalid input: selectors must be an array` | 4 | `pack` |
| `invalid input: query must not be empty` | 1 | `search` |
| `invalid input: invalid selector BenchAuditSink` | 1 | `implementors` |

The `refs.include` and `pack.selectors` failures are addressed by follow-up task `T20260425-0729` (accept scalar string-list inputs across the tool surface).

---

## Failure Taxonomy

Non-passing runs:

| arm | fixture | seeds | classification | observed answer |
|---|---|---|---|---|
| graph-only | `module-surface-orbit-mcp` | 1, 2, 3 | known graph bug / root-surface gap (`T20260425-0739`) | returned `McpHost`, `serve_stdio`; excluded `OrbitToolServer` |
| graph-only | `reverse-export-orbit-error` | 1, 2, 3 | known graph bug / re-export metadata gap (`T20260425-0739`) | returned `[]`; excluded the original definition |

Anomaly flags:

| flag | runs | notes |
|---|---:|---|
| schema-coercion | 25 | 28 failed graph calls total; most were recovered by retrying with array-shaped args. |
| payload-firehose | 6 | Independent flag. Primary taxonomy only shows 2 because schema-coercion wins precedence when both occur. |
| wrong-tool | 6 | The six graph-only non-passing runs above. |
| design-defect | 13 | Hybrid pass with zero graph calls. Interpret as "organic selection avoided graph", not as a correctness failure. |

---

## Standout Fixtures

Top graph-only wins by token reduction:

| fixture | result |
|---|---|
| `deps-downstream-orbit-knowledge` | 3/3 pass, 0.14x no-graph tokens |
| `callers-2hop-graphbenchpolicy` | 3/3 pass, 0.44x no-graph tokens |
| `impl-divergence-trait-method` | 3/3 pass, 0.48x no-graph tokens |

Top graph-only losses:

| fixture | result |
|---|---|
| `reverse-export-orbit-error` | 0/3 pass, 5.59x no-graph tokens |
| `module-surface-orbit-mcp` | 0/3 pass, 1.89x no-graph tokens |
| `construct-vs-match-benchevent-distinct` | 3/3 pass, but 5.41x no-graph tokens |

---

## Interpretation

The Codex v4 result supports keeping graph as an optional navigation surface, not as a replacement for source reads. Graph is excellent when the question maps directly to a graph primitive (`deps`, bounded `callers`, implementor enumeration). Its current export-surface weakness is now tracked as `T20260425-0739`: the parser drops `pub use` re-exports from file `exports` metadata, making graph confidently incomplete on root-surface and re-export-chain queries.

Hybrid is the practical success case: Codex used graph on 11/24 hybrid runs and passed every seed. The winning behavior was selective graph use, not blanket graph use. This suggests the next tool work should focus on making high-confidence graph queries cheaper and more precise, while preserving source fallback.

The highest-leverage fixes before another round are:

1. Implement scalar string-list coercion across graph tools (`T20260425-0729`), which directly addresses 26/28 failed graph calls.
2. Fix `pub use` / re-export indexing and file metadata (`T20260425-0739`), then regenerate the graph index and rerun the affected graph-only fixtures before launching Claude.
3. Add payload shaping for high-cardinality responses (`refs`, `overview`, `callers`) so a single query cannot dump 90k+ response characters.
4. Keep fixture-level ratios as the main cost metric. Aggregate medians hide both the `deps` win and the `reverse-export` loss.

---

## Reproduction

Aggregate tables:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v4/runs \
  --tasks benchmarks/graph/v4/tasks
```

Completed Codex sweeps:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/sweep.py \
  --provider codex --arms no-graph graph-only --n 3
```

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/sweep.py \
  --provider codex --arms hybrid --n 3 \
  --tasks callers-2hop-graphbenchpolicy construct-vs-match-benchevent-distinct \
  deps-downstream-orbit-knowledge function-as-value-vs-direct-call \
  generic-dispatch-concrete-impl implementors-benchsink-with-blanket \
  macro-expanded-callers reverse-export-orbit-error
```

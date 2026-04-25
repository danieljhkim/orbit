# Graph Token-Usage Benchmark - v4 Results

**Status:** Complete for Codex and Claude.
**Sweep dates:** 2026-04-24 to 2026-04-25
**Codex sweep IDs:** `20260424-230632-f36f84` (`no-graph`, `graph-only`), `20260425-001959-842b8c` (`hybrid`)
**Claude sweep IDs:** `20260425-013339-cfac6a` (`no-graph`, `graph-only`), `20260425-012511-8881cb` (`hybrid`)
**Codex sweep seeds:** `928176111` (`no-graph`, `graph-only`), `583229300` (`hybrid`)
**Claude sweep seeds:** `152346771` (`no-graph`, `graph-only`), `88160319` (`hybrid`)
**Harness SHAs:** Codex report baseline `b0ce189e7053409c8754865bd154cd20e1de66a6`; Claude post-fix run/report update `65fc1a22888ac120c8185c891d23cc5c7bf1c07e`
**Scope:** 192 cells total. Each provider ran `no-graph` 12 fixtures x 3 seeds, `graph-only` 12 fixtures x 3 seeds, and `hybrid` 8 fixtures x 3 seeds. No errored cells.

**Important comparison caveat:** Codex is a pre-fix baseline. Claude ran after `T20260425-0729` (string-list input coercion) and `T20260425-0739` (pub-use re-export indexing) landed and the graph index was rebuilt. Treat Codex-vs-Claude deltas as diagnostic evidence about model behavior plus tool fixes, not a clean provider-only comparison.

---

## Headline

1. **Codex pre-fix:** `no-graph` passed 36/36, `graph-only` passed 30/36, and `hybrid` passed 24/24. All Codex graph-only failures came from `module-surface-orbit-mcp` and `reverse-export-orbit-error`, the two fixtures most exposed to the `T20260425-0739` re-export bug.
2. **Claude post-fix:** `graph-only` passed 36/36, `hybrid` passed 24/24, and `no-graph` passed 34/36. The two Claude no-graph failures were both `const-value-extraction` runs that omitted `V2_TOOL_WILDCARD_ROOTS`.
3. **The re-export fix appears load-bearing:** after `T20260425-0739`, Claude graph-only passed `reverse-export-orbit-error` and `module-surface-orbit-mcp` in all seeds. That does not retroactively make the Codex run comparable, but it strongly supports the earlier diagnosis that those Codex failures were graph metadata bugs.
4. **Graph-only accuracy improved post-fix, but cost remains uneven:** Claude graph-only was perfect but costlier than Claude no-graph on 10/12 fixtures. Its only fixture-level token wins were `deps-downstream-orbit-knowledge` (0.83x no-graph) and `references-vs-callers-tool-registry-register` (0.89x).
5. **Hybrid remains the practical operating mode, but providers route differently:** Codex hybrid used graph in 11/24 runs and passed all seeds. Claude hybrid used graph in only 3/24 runs, all on `deps-downstream-orbit-knowledge`, and passed the rest via shell/source fallback.
6. **The remaining graph work is payload shaping and argument ergonomics:** after scalar string-list coercion landed, Claude still hit 9 failed graph calls from nested-list/invalid-selector shapes. Claude graph-only also triggered 7 primary payload-firehose flags.

---

## Arm Summary

Token totals are `input_tokens + output_tokens`, matching the aggregator's marginal-token convention. Cached read tokens and Claude USD cost are reported separately by the raw records, but not included in the median-token columns.

| provider | arm | runs | pass | median_total_tokens | p90_total_tokens | graph_call_rate | graph_calls | failed_graph_calls | shell_or_fs_calls |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| claude | no-graph | 36 | 34/36 | 713 | 3159 | 0/36 | 0 | 0 | 154 |
| claude | graph-only | 36 | 36/36 | 2330 | 6866 | 36/36 | 436 | 9 | 0 |
| claude | hybrid | 24 | 24/24 | 663 | 2449 | 3/24 | 3 | 0 | 45 |
| codex | no-graph | 36 | 36/36 | 11446 | 27792 | 0/36 | 0 | 0 | 197 |
| codex | graph-only | 36 | 30/36 | 15462 | 64877 | 36/36 | 334 | 25 | 0 |
| codex | hybrid | 24 | 24/24 | 3900 | 11048 | 11/24 | 40 | 3 | 61 |

Hybrid only ran on the 8 graph-strength and precision-gap fixtures, per `METHOD.md`. On that same 24-run subset:

| provider | no-graph median | graph-only median | hybrid median |
|---|---:|---:|---:|
| claude | 491 | 1541 | 663 |
| codex | 11446 | 15114 | 3900 |

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
| claude | graph-only | graph-strength | 12 | 100% | 1310 | 3944 | 1871 | 77 | 12/12 = 100.0% | 0 |
| claude | graph-only | payload-volume | 6 | 100% | 5128 | 11584 | 5636 | 201 | 6/6 = 100.0% | 0 |
| claude | graph-only | precision-gap | 12 | 100% | 1854 | 4538 | 2252 | 90 | 12/12 = 100.0% | 0 |
| claude | graph-only | selector-ambiguity | 6 | 100% | 3960 | 7645 | 4308 | 68 | 6/6 = 100.0% | 0 |
| claude | hybrid | graph-strength | 12 | 100% | 669 | 1842 | 818 | 3 | 3/12 = 25.0% | 12 |
| claude | hybrid | precision-gap | 12 | 100% | 468 | 2887 | 935 | 0 | 0/12 = 0.0% | 33 |
| claude | no-graph | graph-strength | 12 | 100% | 536 | 1841 | 737 | 0 | 0/12 = 0.0% | 31 |
| claude | no-graph | payload-volume | 6 | 67% | 1517 | 2508 | 2207 | 0 | 0/6 = 0.0% | 34 |
| claude | no-graph | precision-gap | 12 | 100% | 343 | 3052 | 874 | 0 | 0/12 = 0.0% | 30 |
| claude | no-graph | selector-ambiguity | 6 | 100% | 2677 | 5296 | 3188 | 0 | 0/6 = 0.0% | 59 |
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

`median go/ng` compares aggregate category medians. `mean fixture go/ng` and `worst fixture go/ng` are per-fixture ratios and are the load-bearing cost readings. Lower ratios are better for graph-only.

| provider | category | no-graph pass | graph-only pass | pass delta | median go/ng | mean fixture go/ng | worst fixture go/ng | hybrid graph rate |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| claude | graph-strength | 12/12 | 12/12 | +0 | 2.44x | 3.72x | 6.73x | 3/12 |
| claude | precision-gap | 12/12 | 12/12 | +0 | 5.41x | 4.65x | 7.61x | 0/12 |
| claude | payload-volume | 4/6 | 6/6 | +2 | 3.38x | 5.68x | 9.71x | n/a |
| claude | selector-ambiguity | 6/6 | 6/6 | +0 | 1.48x | 1.42x | 1.94x | n/a |
| codex | graph-strength | 12/12 | 9/12 | -3 | 0.71x | 1.70x | 5.59x | 9/12 |
| codex | precision-gap | 12/12 | 12/12 | +0 | 1.38x | 2.15x | 5.41x | 2/12 |
| codex | payload-volume | 6/6 | 6/6 | +0 | 1.36x | 1.87x | 3.25x | n/a |
| codex | selector-ambiguity | 6/6 | 3/6 | -3 | 1.14x | 1.45x | 1.89x | n/a |

---

## Production vs Synthetic

| provider | mode | arm | runs | pass | median_total_tokens | graph_call_rate | graph_calls | failed_graph_calls |
|---|---|---|---:|---:|---:|---:|---:|---:|
| claude | production | no-graph | 21 | 19/21 | 1916 | 0/21 | 0 | 0 |
| claude | production | graph-only | 21 | 21/21 | 3720 | 21/21 | 357 | 6 |
| claude | production | hybrid | 9 | 9/9 | 1735 | 3/9 | 3 | 0 |
| claude | synthetic | no-graph | 15 | 15/15 | 219 | 0/15 | 0 | 0 |
| claude | synthetic | graph-only | 15 | 15/15 | 1283 | 15/15 | 79 | 3 |
| claude | synthetic | hybrid | 15 | 15/15 | 318 | 0/15 | 0 | 0 |
| codex | production | no-graph | 21 | 21/21 | 14162 | 0/21 | 0 | 0 |
| codex | production | graph-only | 21 | 15/21 | 17847 | 21/21 | 222 | 18 |
| codex | production | hybrid | 9 | 9/9 | 4872 | 3/9 | 3 | 0 |
| codex | synthetic | no-graph | 15 | 15/15 | 11278 | 0/15 | 0 | 0 |
| codex | synthetic | graph-only | 15 | 15/15 | 14972 | 15/15 | 112 | 7 |
| codex | synthetic | hybrid | 15 | 15/15 | 3819 | 8/15 | 37 | 3 |

The production split is the load-bearing product signal. Codex graph-only lost accuracy only on production-grounded fixtures. Claude graph-only passed all production fixtures post-fix, while Claude no-graph missed `const-value-extraction` twice.

---

## Claude Per-Fixture Table

| fixture | class | mode | arm | pass | median_tokens | p90_tokens | graph_call_rate | graph_calls | failed_graph_calls | shell/fs_calls |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | no-graph | 3/3 | 535 | 746 | 0/3 | 0 | 0 | 3 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | graph-only | 3/3 | 1016 | 1283 | 3/3 | 6 | 0 | 0 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | hybrid | 3/3 | 750 | 855 | 0/3 | 0 | 0 | 3 |
| `const-value-extraction` | payload-volume | production | no-graph | 1/3 | 719 | 1198 | 0/3 | 0 | 0 | 14 |
| `const-value-extraction` | payload-volume | production | graph-only | 3/3 | 6979 | 11584 | 3/3 | 158 | 0 | 0 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | no-graph | 3/3 | 481 | 699 | 0/3 | 0 | 0 | 3 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | graph-only | 3/3 | 2290 | 2370 | 3/3 | 30 | 0 | 0 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | hybrid | 3/3 | 719 | 732 | 0/3 | 0 | 0 | 3 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | no-graph | 3/3 | 1610 | 1940 | 0/3 | 0 | 0 | 19 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | graph-only | 3/3 | 1338 | 1959 | 3/3 | 3 | 0 | 0 |
| `deps-downstream-orbit-knowledge` | graph-strength | production | hybrid | 3/3 | 1735 | 1888 | 3/3 | 3 | 0 | 0 |
| `function-as-value-vs-direct-call` | precision-gap | production | no-graph | 3/3 | 2308 | 3371 | 0/3 | 0 | 0 | 21 |
| `function-as-value-vs-direct-call` | precision-gap | production | graph-only | 3/3 | 4273 | 4652 | 3/3 | 24 | 0 | 0 |
| `function-as-value-vs-direct-call` | precision-gap | production | hybrid | 3/3 | 2823 | 2915 | 0/3 | 0 | 0 | 24 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | no-graph | 3/3 | 197 | 219 | 0/3 | 0 | 0 | 3 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | graph-only | 3/3 | 864 | 1196 | 3/3 | 13 | 0 | 0 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | hybrid | 3/3 | 219 | 239 | 0/3 | 0 | 0 | 3 |
| `impl-divergence-trait-method` | payload-volume | production | no-graph | 3/3 | 2004 | 2508 | 0/3 | 0 | 0 | 20 |
| `impl-divergence-trait-method` | payload-volume | production | graph-only | 3/3 | 3332 | 3438 | 3/3 | 43 | 0 | 0 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | no-graph | 3/3 | 184 | 184 | 0/3 | 0 | 0 | 3 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | graph-only | 3/3 | 998 | 1537 | 3/3 | 7 | 0 | 0 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | hybrid | 3/3 | 318 | 327 | 0/3 | 0 | 0 | 3 |
| `macro-expanded-callers` | precision-gap | synthetic | no-graph | 3/3 | 203 | 231 | 0/3 | 0 | 0 | 3 |
| `macro-expanded-callers` | precision-gap | synthetic | graph-only | 3/3 | 1545 | 1558 | 3/3 | 23 | 3 | 0 |
| `macro-expanded-callers` | precision-gap | synthetic | hybrid | 3/3 | 203 | 219 | 0/3 | 0 | 0 | 3 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | no-graph | 3/3 | 1916 | 2285 | 0/3 | 0 | 0 | 10 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | graph-only | 3/3 | 3720 | 4383 | 3/3 | 28 | 1 | 0 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | no-graph | 3/3 | 4698 | 5296 | 0/3 | 0 | 0 | 49 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | graph-only | 3/3 | 4201 | 7645 | 3/3 | 40 | 1 | 0 |
| `reverse-export-orbit-error` | graph-strength | production | no-graph | 3/3 | 537 | 707 | 0/3 | 0 | 0 | 6 |
| `reverse-export-orbit-error` | graph-strength | production | graph-only | 3/3 | 3613 | 4086 | 3/3 | 61 | 4 | 0 |
| `reverse-export-orbit-error` | graph-strength | production | hybrid | 3/3 | 588 | 629 | 0/3 | 0 | 0 | 6 |

## Codex Per-Fixture Table

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

## Claude Hybrid Utilization

| fixture | pass | median_tokens | graph_call_rate | graph_calls | shell/fs_calls | interpretation |
|---|---:|---:|---:|---:|---:|---|
| `callers-2hop-graphbenchpolicy` | 3/3 | 750 | 0/3 | 0 | 3 | Passed by shell/source fallback; graph avoided organically. |
| `construct-vs-match-benchevent-distinct` | 3/3 | 719 | 0/3 | 0 | 3 | Passed by shell/source fallback; graph avoided organically. |
| `deps-downstream-orbit-knowledge` | 3/3 | 1735 | 3/3 | 3 | 0 | Only Claude hybrid fixture that used graph; direct deps solved it cleanly. |
| `function-as-value-vs-direct-call` | 3/3 | 2823 | 0/3 | 0 | 24 | Passed by source fallback with relatively heavy shell/read use. |
| `generic-dispatch-concrete-impl` | 3/3 | 219 | 0/3 | 0 | 3 | Passed by direct source inspection; graph avoided organically. |
| `implementors-benchsink-with-blanket` | 3/3 | 318 | 0/3 | 0 | 3 | Passed by direct source inspection; graph avoided organically. |
| `macro-expanded-callers` | 3/3 | 203 | 0/3 | 0 | 3 | Passed by direct source inspection; graph avoided organically. |
| `reverse-export-orbit-error` | 3/3 | 588 | 0/3 | 0 | 6 | Passed by shell/source fallback despite graph-only success post-fix. |

Claude hybrid shows that neutral hybrid prompting does not guarantee graph selection. It mostly measures whether Claude can route to the cheapest available source strategy, and for this fixture set that was usually shell/source reading rather than graph.

## Codex Hybrid Utilization

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

Codex hybrid's 24/24 pass rate is not proof that every hybrid-eligible fixture is graph-shaped. It is proof that Codex can route around graph gaps when shell/source tools are available.

---

## Claude Graph-Only Cost Ratios

| fixture | class | mode | no-graph median | graph-only median | go/ng | graph-only pass |
|---|---|---|---:|---:|---:|---:|
| `deps-downstream-orbit-knowledge` | graph-strength | production | 1610 | 1338 | 0.83x | 3/3 |
| `references-vs-callers-tool-registry-register` | selector-ambiguity | production | 4698 | 4201 | 0.89x | 3/3 |
| `impl-divergence-trait-method` | payload-volume | production | 2004 | 3332 | 1.66x | 3/3 |
| `function-as-value-vs-direct-call` | precision-gap | production | 2308 | 4273 | 1.85x | 3/3 |
| `callers-2hop-graphbenchpolicy` | graph-strength | synthetic | 535 | 1016 | 1.90x | 3/3 |
| `module-surface-orbit-mcp` | selector-ambiguity | production | 1916 | 3720 | 1.94x | 3/3 |
| `generic-dispatch-concrete-impl` | precision-gap | synthetic | 197 | 864 | 4.39x | 3/3 |
| `construct-vs-match-benchevent-distinct` | precision-gap | synthetic | 481 | 2290 | 4.76x | 3/3 |
| `implementors-benchsink-with-blanket` | graph-strength | synthetic | 184 | 998 | 5.42x | 3/3 |
| `reverse-export-orbit-error` | graph-strength | production | 537 | 3613 | 6.73x | 3/3 |
| `macro-expanded-callers` | precision-gap | synthetic | 203 | 1545 | 7.61x | 3/3 |
| `const-value-extraction` | payload-volume | production | 719 | 6979 | 9.71x | 3/3 |

Claude graph-only was excellent for accuracy after the graph fixes, but it was rarely the cheapest route. The `const-value-extraction` cell is the sharpest tradeoff: graph-only found the full set in every seed, while no-graph missed one constant twice, but graph-only used 9.71x the no-graph median tokens.

## Codex Graph-Only Cost Ratios

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

## Claude Tool Diagnostics

Per-tool response size is measured in response characters, not model tokens. The transcripts do not expose per-tool token attribution.

| tool | invocations | succeeded | failed | success_rate | median_response_chars | p90_response_chars |
|---|---:|---:|---:|---:|---:|---:|
| callers | 7 | 4 | 3 | 57% | 1838 | 29660 |
| refs | 14 | 10 | 4 | 71% | 3945 | 18990 |
| implementors | 6 | 6 | 0 | 100% | 1122 | 1249 |
| deps | 6 | 6 | 0 | 100% | 606 | 606 |
| pack | 2 | 1 | 1 | 50% | 979 | 979 |
| search | 80 | 80 | 0 | 100% | 212 | 1427 |
| show | 324 | 323 | 1 | 100% | 762 | 2931 |
| overview | 0 | 0 | 0 | n/a | - | - |

Failed graph calls by message:

| message | count | affected tools |
|---|---:|---|
| `invalid input: include entries must be code, doc, config, or all, got ["code"]` | 3 | `refs` |
| `execution failed: selector BenchDerivedStruct::default:fn does not resolve to a node` | 2 | `callers` |
| `execution failed: selector BenchDerivedStruct::default does not resolve to a node` | 1 | `callers` |
| `invalid input: invalid selector ["file:crates/orbit-mcp/src/lib.rs"]` | 1 | `pack` |
| `invalid input: selector file:crates/orbit-common/src/types.rs does not resolve to a node` | 1 | `show` |
| `invalid input: include entries must be code, doc, config, or all, got ["code", "config"]` | 1 | `refs` |

These failures are not the same shape as the pre-fix Codex scalar-list failures. The remaining Claude failures are nested-list/invalid-selector mistakes and a derive/default selector expectation that graph does not support.

## Codex Tool Diagnostics

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

The `refs.include` and `pack.selectors` scalar-list failures were addressed by `T20260425-0729` before the Claude sweep.

---

## Failure Taxonomy

Non-passing runs:

| provider | arm | fixture | seeds | classification | observed answer |
|---|---|---|---|---|---|
| claude | no-graph | `const-value-extraction` | 1, 2 | source-search miss | omitted `V2_TOOL_WILDCARD_ROOTS`; seed 3 found the full set |
| codex | graph-only | `module-surface-orbit-mcp` | 1, 2, 3 | known graph bug / root-surface gap (`T20260425-0739`) | returned `McpHost`, `serve_stdio`; excluded `OrbitToolServer` |
| codex | graph-only | `reverse-export-orbit-error` | 1, 2, 3 | known graph bug / re-export metadata gap (`T20260425-0739`) | returned `[]`; excluded the original definition |

Anomaly flags are not mutually exclusive. `Primary` means the row count emitted by `aggregate.py`'s precedence-ordered taxonomy; `independent` means the flag was true even if another flag won precedence.

| provider | flag | runs | notes |
|---|---|---:|---|
| claude | schema-coercion | 8 primary | 9 failed graph calls total; all recovered. Remaining shapes are nested-list/invalid-selector errors, not the pre-fix scalar-list issue. |
| claude | payload-firehose | 7 primary / 13 independent | Concentrated in graph-only `const-value-extraction`, `macro-expanded-callers`, `reverse-export-orbit-error`, `implementors-benchsink-with-blanket`, and one `generic-dispatch-concrete-impl` seed. |
| claude | wrong-tool | 0 | No graph-only Claude run failed. |
| claude | design-defect | 21 | Hybrid passed with zero graph calls. Interpret as "organic selection avoided graph", not as a correctness failure. |
| codex | schema-coercion | 25 primary | 28 failed graph calls total; most were recovered by retrying with array-shaped args. |
| codex | payload-firehose | 2 primary / 6 independent | Primary taxonomy hides several firehose runs behind schema-coercion. |
| codex | wrong-tool | 6 | The six pre-fix graph-only non-passing runs above. |
| codex | design-defect | 13 | Hybrid passed with zero graph calls. Interpret as "organic selection avoided graph", not as a correctness failure. |

---

## Standout Fixtures

Top graph-only wins by token reduction:

| provider | fixture | result |
|---|---|---|
| codex | `deps-downstream-orbit-knowledge` | 3/3 pass, 0.14x no-graph tokens |
| codex | `callers-2hop-graphbenchpolicy` | 3/3 pass, 0.44x no-graph tokens |
| codex | `impl-divergence-trait-method` | 3/3 pass, 0.48x no-graph tokens |
| claude | `deps-downstream-orbit-knowledge` | 3/3 pass, 0.83x no-graph tokens |
| claude | `references-vs-callers-tool-registry-register` | 3/3 pass, 0.89x no-graph tokens |

Accuracy standouts:

| provider | fixture | result |
|---|---|---|
| claude | `reverse-export-orbit-error` | graph-only 3/3 post-fix; Codex pre-fix graph-only was 0/3 |
| claude | `module-surface-orbit-mcp` | graph-only 3/3 post-fix; Codex pre-fix graph-only was 0/3 |
| claude | `const-value-extraction` | graph-only 3/3 while no-graph was 1/3 |

Top graph-only losses:

| provider | fixture | result |
|---|---|---|
| claude | `const-value-extraction` | 3/3 pass, but 9.71x no-graph tokens |
| claude | `macro-expanded-callers` | 3/3 pass, but 7.61x no-graph tokens |
| claude | `reverse-export-orbit-error` | 3/3 pass post-fix, but 6.73x no-graph tokens |
| codex | `reverse-export-orbit-error` | 0/3 pass pre-fix, 5.59x no-graph tokens |
| codex | `construct-vs-match-benchevent-distinct` | 3/3 pass, but 5.41x no-graph tokens |

---

## Interpretation

The full v4 result supports keeping graph as an optional navigation surface, not as a replacement for source reads. Graph is excellent when the question maps directly to a precise graph primitive, with `deps-downstream-orbit-knowledge` the cleanest repeated win across both providers.

`T20260425-0739` looks like the right diagnosis for the Codex graph-only failures: after the pub-use/re-export fix, Claude graph-only passed both `reverse-export-orbit-error` and `module-surface-orbit-mcp`. That said, Claude's post-fix run shows a different problem: graph-only can be correct and still too expensive, especially when agents enumerate through many `show` calls or use graph for source-body extraction tasks.

Hybrid is still the practical success case, but its meaning differs by provider. Codex selectively used graph and got the strongest overall cost/correctness profile on the hybrid subset. Claude mostly avoided graph in hybrid, so its 24/24 result is better read as "source fallback remains essential" than "graph was selected well."

The highest-leverage next steps are:

1. Add payload shaping for high-cardinality responses (`refs`, `overview`, `callers`, and repeated `show`) so graph-only cannot spend 6x-10x tokens on enumeration.
2. Tighten selector and argument affordances after `T20260425-0729`: scalar lists are fixed, but nested lists and JSON-array-as-string selectors still cause recoverable failures.
3. Rerun Codex graph-only on the two re-export/root-surface fixtures, or rerun the full Codex graph-only arm post-fix, to separate provider behavior from the `T20260425-0739` tool fix.
4. Add a small hybrid-selection round with explicit "prefer graph when it directly answers the relationship; fall back to source for bodies/values" guidance. Neutral hybrid prompts measure organic tool choice, and Claude's organic choice was mostly "do not use graph."
5. Keep fixture-level ratios as the main cost metric. Aggregate medians hide both the `deps` win and the expensive-but-correct post-fix `reverse-export` result.

---

## Reproduction

Aggregate tables:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v4/runs \
  --tasks benchmarks/graph/v4/tasks
```

Completed Codex sweeps:

These artifacts are pre-fix for `T20260425-0729` and `T20260425-0739`; rerunning at current HEAD will not reproduce the exact Codex graph-only failures.

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

Completed Claude sweeps:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/sweep.py \
  --provider claude --arms no-graph graph-only --n 3
```

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/sweep.py \
  --provider claude --arms hybrid --n 3 \
  --tasks callers-2hop-graphbenchpolicy construct-vs-match-benchevent-distinct \
  deps-downstream-orbit-knowledge function-as-value-vs-direct-call \
  generic-dispatch-concrete-impl implementors-benchsink-with-blanket \
  macro-expanded-callers reverse-export-orbit-error
```

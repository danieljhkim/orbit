# Graph Token-Usage Benchmark — v3 Results

**Task:** v3 graph MCP parity sweep ([T20260423-0524](./METHOD.md#task-references))
**Sweep date:** 2026-04-23
**Sweep IDs:** `20260423-194444-3b15a2` (claude), `20260423-210616-9a7e7c` (codex)
**Harness SHA at report write:** `a37f95cecca7d22711a4b47d9fddb7efed2a0f3b`
**Scope:** 2 providers × 3 arms × 10 fixtures × 3 seeds = 180 cells. Three errored cells are excluded from aggregate tables.
**Providers:** Claude (`claude-sonnet-4-6`) and Codex (`gpt-5.3-codex`).
**Fixtures:** `callers-run-deterministic-containers`, `deps-orbit-knowledge-consumers`, `impact-scope-strategy-callsites`, `impact-tool-context-struct-literals`, `locate-agentruntime`, `locate-loopaudit-variants`, `locate-v2-runtime-host-trait`, `trace-policy-denial-wiring`, `trace-tool-call-event-construct-sites`, `trace-v2runtime-production-impls`.

---

## Headline

1. **Codex hybrid utilization flipped once MCP parity landed.** The v2→v3 graph-use rate moved **0/30 → 23/30**. v3 codex hybrid made 79 graph calls.
2. **Claude still did not organically select graph.** Claude hybrid made **0 graph calls in 30 runs**, even though graph-only proves the tools were available.
3. **Codex accuracy improved under graph access.** Codex pass rates were no-graph 28/30, graph-only **30/30**, hybrid 29/30.
4. **Cost remains mixed under the strict per-cell reading.** Codex graph-only clears the 1.3× no-graph threshold on 4/10 fixtures; Claude clears it on 1/10.
5. **The firehose problem is real.** `impact-tool-context-struct-literals` is a 12.43× codex graph-only outlier; one passing graph run made 17 `pack` calls to assemble what no-graph solved far cheaper.

---

## Tool-utilization audit

Per-arm counts from the frozen run records' `tool_calls` histograms.

| provider / arm | non-error runs | runs_with_graph_call | graph_calls | shell_or_fs_calls |
|---|---:|---:|---:|---:|
| claude / no-graph | 29 | 0 | 0 | 100 |
| claude / hybrid | 30 | 0 | 0 | 93 |
| claude / graph-only | 29 | 29 | 350 | 0 |
| codex / no-graph | 29 | 0 | 0 | 219 |
| codex / hybrid | 30 | **23** | **79** | 148 |
| codex / graph-only | 30 | 30 | 468 | 0 |

Codex hybrid graph-call rate by fixture:

| fixture | runs_with_graph_call | graph_calls | pass_rate |
|---|---:|---:|---:|
| callers-run-deterministic-containers | 2/3 | 10 | 3/3 |
| deps-orbit-knowledge-consumers | 3/3 | 3 | 3/3 |
| impact-scope-strategy-callsites | 0/3 | 0 | 3/3 |
| impact-tool-context-struct-literals | 1/3 | 3 | 3/3 |
| locate-agentruntime | 3/3 | 11 | 3/3 |
| locate-loopaudit-variants | 2/3 | 4 | 3/3 |
| locate-v2-runtime-host-trait | 3/3 | 6 | 3/3 |
| trace-policy-denial-wiring | 3/3 | 18 | 3/3 |
| trace-tool-call-event-construct-sites | 3/3 | 16 | 2/3 |
| trace-v2runtime-production-impls | 3/3 | 8 | 3/3 |

The `impact-scope-strategy-callsites` 0/3 codex-hybrid outlier is not a graph loss: it is a 4-file grep-ergonomic task, and codex passed all three seeds without graph.

---

## Primary table

Verbatim from:

```bash
GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v3/runs --tasks benchmarks/graph/v3/tasks
```

| provider | arm | task_class | runs | pass_rate | median_total_tokens | p90_total_tokens | tokens_per_success | graph_calls | graph_call_rate | shell_or_fs_calls |
|---|---|---|---|---|---|---|---|---|---|---|
| claude | graph-only | deps | 3 | 100% | 1221 | 1550 | 1245 | 17 | 3/3 = 100.0% | 0 |
| claude | graph-only | impact | 6 | 50% | 6022 | 11912 | 14284 | 190 | 6/6 = 100.0% | 0 |
| claude | graph-only | locate | 8 | 100% | 718 | 938 | 742 | 22 | 8/8 = 100.0% | 0 |
| claude | graph-only | trace | 12 | 75% | 3426 | 5691 | 3968 | 121 | 12/12 = 100.0% | 0 |
| claude | hybrid | deps | 3 | 100% | 292 | 559 | 378 | 0 | 0/3 = 0.0% | 3 |
| claude | hybrid | impact | 6 | 100% | 776 | 2298 | 1092 | 0 | 0/6 = 0.0% | 24 |
| claude | hybrid | locate | 9 | 100% | 438 | 909 | 416 | 0 | 0/9 = 0.0% | 18 |
| claude | hybrid | trace | 12 | 83% | 1036 | 2441 | 1494 | 0 | 0/12 = 0.0% | 48 |
| claude | no-graph | deps | 3 | 100% | 315 | 669 | 425 | 0 | 0/3 = 0.0% | 5 |
| claude | no-graph | impact | 6 | 100% | 896 | 2390 | 1124 | 0 | 0/6 = 0.0% | 25 |
| claude | no-graph | locate | 8 | 100% | 449 | 705 | 414 | 0 | 0/8 = 0.0% | 18 |
| claude | no-graph | trace | 12 | 92% | 954 | 2601 | 1359 | 0 | 0/12 = 0.0% | 52 |
| codex | graph-only | deps | 3 | 100% | 10147 | 41682 | 19321 | 38 | 3/3 = 100.0% | 0 |
| codex | graph-only | impact | 6 | 100% | 55376 | 318655 | 114800 | 115 | 6/6 = 100.0% | 0 |
| codex | graph-only | locate | 9 | 100% | 14904 | 28650 | 14801 | 43 | 9/9 = 100.0% | 0 |
| codex | graph-only | trace | 12 | 100% | 40187 | 92250 | 43638 | 272 | 12/12 = 100.0% | 0 |
| codex | hybrid | deps | 3 | 100% | 16138 | 17352 | 16311 | 3 | 3/3 = 100.0% | 14 |
| codex | hybrid | impact | 6 | 100% | 11175 | 33501 | 14987 | 3 | 1/6 = 16.7% | 40 |
| codex | hybrid | locate | 9 | 100% | 14018 | 15737 | 12424 | 21 | 8/9 = 88.9% | 14 |
| codex | hybrid | trace | 12 | 92% | 21232 | 52050 | 25949 | 52 | 11/12 = 91.7% | 80 |
| codex | no-graph | deps | 3 | 100% | 12416 | 12933 | 12516 | 0 | 0/3 = 0.0% | 15 |
| codex | no-graph | impact | 6 | 100% | 15916 | 19787 | 15773 | 0 | 0/6 = 0.0% | 55 |
| codex | no-graph | locate | 9 | 89% | 23253 | 37795 | 25322 | 0 | 0/9 = 0.0% | 54 |
| codex | no-graph | trace | 11 | 100% | 26184 | 37689 | 25940 | 0 | 0/11 = 0.0% | 95 |

---

## Cost

USD totals are available for Claude only. Codex reports `$0.0000` because the Codex CLI does not emit billing, not because usage was free.

| provider / arm | cost_usd |
|---|---:|
| claude / no-graph | $1.9657 |
| claude / hybrid | $1.9091 |
| claude / graph-only | $4.3260 |
| codex / no-graph | $0.0000 |
| codex / hybrid | $0.0000 |
| codex / graph-only | $0.0000 |

The pre-registered cost criterion is token-based and per-cell: graph-only median `(input + output)` tokens must be ≤ 1.3× the matching no-graph median for the same provider × fixture cell.

| fixture | codex go/ng | codex ≤1.3× | claude go/ng | claude ≤1.3× |
|---|---:|:---:|---:|:---:|
| deps-orbit-knowledge-consumers | 0.82× | yes | 3.88× | no |
| locate-agentruntime | 0.44× | yes | 1.80× | no |
| locate-v2-runtime-host-trait | 0.18× | yes | 1.57× | no |
| trace-v2runtime-production-impls | 0.58× | yes | 0.77× | yes |
| trace-policy-denial-wiring | 1.74× | no | 1.46× | no |
| locate-loopaudit-variants | 4.87× | no | 3.88× | no |
| callers-run-deterministic-containers | 1.67× | no | 5.10× | no |
| trace-tool-call-event-construct-sites | 2.08× | no | 2.74× | no |
| impact-scope-strategy-callsites | 1.85× | no | 27.18× | no |
| impact-tool-context-struct-literals | **12.43×** | no | 2.82× | no |
| **per-cell pass count** | **4 / 10** | | **1 / 10** | |

Aggregate medians are secondary: codex graph-only is 1.09× no-graph across all fixtures, while Claude graph-only is 1.44×. The strict per-cell reading is the load-bearing result.

---

## Pass-rate breakdown

Non-passing cells:

| provider | arm | fixture | seed | verdict | diagnostic |
|---|---|---|---:|---|---|
| claude | graph-only | callers-run-deterministic-containers | 2 | fail | oracle rejected final message |
| claude | graph-only | impact-tool-context-struct-literals | 1 | fail | oracle rejected final message |
| claude | graph-only | impact-tool-context-struct-literals | 2 | fail | oracle rejected final message |
| claude | graph-only | impact-tool-context-struct-literals | 3 | fail | oracle rejected final message |
| claude | graph-only | trace-tool-call-event-construct-sites | 1 | fail | oracle rejected final message |
| claude | graph-only | trace-tool-call-event-construct-sites | 2 | fail | oracle rejected final message |
| claude | hybrid | trace-policy-denial-wiring | 2 | fail | oracle rejected final message |
| claude | hybrid | trace-policy-denial-wiring | 3 | fail | oracle rejected final message |
| claude | no-graph | trace-policy-denial-wiring | 1 | fail | oracle rejected final message |
| codex | hybrid | trace-tool-call-event-construct-sites | 2 | fail | oracle rejected final message |
| codex | no-graph | locate-v2-runtime-host-trait | 1 | fail | oracle rejected final message |

Errored cells excluded from aggregate tables:

| provider | arm | fixture | seed | diagnostic |
|---|---|---|---:|---|
| claude | no-graph | locate-loopaudit-variants | 2 | claude run reported is_error=True: 529 |
| claude | graph-only | locate-agentruntime | 2 | claude run reported is_error=True: 529 |
| codex | no-graph | trace-policy-denial-wiring | 2 | codex produced no parseable result (exit=124) |

Manual audit found several v3 oracle rejections that are better treated as grader artifacts: the substring oracle rejects answers that mention excluded paths as excluded. v4 replaces that with a structured `{"answer": [...], "excluded": [...]}` oracle.

---

## Re-interpretation And Disposition

v3's `METHOD.md` pre-registered the cull threshold:

1. Hybrid utilization ≥ 20% on at least one provider.
2. Graph-only median `(input + output)` tokens ≤ 1.3× the matching no-graph median for the same provider × fixture cell.

Criterion 1 passes on a provider-any reading because codex hybrid used graph in 23/30 runs. Claude fails that criterion at 0/30.

Criterion 2 is mixed and does not clear as a clean sweep: codex passes 4/10 fixture cells, Claude passes 1/10. The cost failures cluster on callers, impact, and trace-construction fixtures, which are exactly where the current graph surface either over-includes by signature/name or hands the agent too much payload.

**Disposition:** retain the agent-facing `orbit_graph_*` MCP surface, carried primarily by codex utilization and accuracy. This is not a clean benchmark pass; it is a product call that the surface is useful for shell-first providers while still needing payload and precision work.

---

## Hypothesis reconciliation

| hypothesis / threshold | result |
|---|---|
| Codex hybrid 0/30 utilization in v2 was a tool-surface asymmetry. | Supported. MCP parity flips codex hybrid to 23/30 graph-use runs. |
| Claude will organically use graph once v3 closes the cross-provider setup. | Falsified. Claude hybrid remains 0/30. |
| Graph-only cost can stay within 1.3× no-graph per provider × fixture cell. | Mostly falsified. Codex passes 4/10; Claude passes 1/10. |
| Graph access can improve codex accuracy on grep-hard fixtures. | Supported. Codex graph-only is 30/30; no-graph is 28/30. |
| Payload volume is an important remaining failure mode. | Supported. `impact-tool-context-struct-literals` reaches 12.43× under codex graph-only. |

---

## Recommendations

1. **Keep the MCP graph surface enabled.** Codex uses it heavily once offered as first-class MCP tools.
2. **Treat provider behavior as part of the product decision.** Claude pays schema/context cost without selecting graph under hybrid; codex gets real navigation value.
3. **Make v4 diagnostic, not keep/cull.** v3 already settles retention. v4 should isolate payload firehose, signature-vs-type precision gaps, selector ambiguity, and graph-strength cases.
4. **Replace substring grading.** v3 oracle artifacts are noisy enough that v4 needs structured answer/excluded grading.
5. **Measure per-cell, not only aggregate medians.** The aggregate codex 1.09× cost hides a 12.43× fixture outlier.

---

## Methodology notes

- **Token accounting:** `median_total_tokens` is `input_tokens + output_tokens`; cached input is reported separately in the secondary aggregate output.
- **Codex billing:** Codex cost remains `$0.0000` because the provider normalizer has no billing feed.
- **Aggregate reproduction:** `GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/aggregate.py --runs benchmarks/graph/v3/runs --tasks benchmarks/graph/v3/tasks`.
- **Fixture-level utilization table:** derived directly from frozen run records' `tool_calls` fields.
- **Known caveats carried into v4:** structured oracle, per-cell threshold specification, per-tool payload diagnostics, and failure taxonomy are all direct responses to v3's residual ambiguity.

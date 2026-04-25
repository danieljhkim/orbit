# v4 — Method & Caveats

This file documents how the `graph_v4` sweep will be conducted. The report lands at `RESULTS.md` in this directory when the round freezes. Conventions governing this document's shape are in `benchmarks/CONVENTIONS.md`.

v4 is explicitly scoped as a **diagnostic round**, not a keep/cull round. v3 settled retention of the agent-facing `orbit_graph_*` MCP surface (see [`../../../docs/design/knowledge-graph/5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md) §"Disposition"). v4 maps where the surface helps, where it hurts, and how it fails — with measured targets for future tool-shaping work (payload trimming, ref-kind precision, schema ergonomics).

## Harness git SHA at freeze

_TBD — populated at freeze._

## Frame: diagnostic, not gating

There is no v4 keep/cull threshold. There IS a pre-registered **report shape** (below) so the round produces structured findings rather than post-hoc cherry-pick.

## Central intervention vs v3

**Experimental knobs are held constant.** Same models (`gpt-5.3-codex`, `claude-sonnet-4-6`), same MCP surface (8 `orbit_graph_*` tools, descriptions unchanged), same sandbox/elicitation settings. The single experimental change is the **fixture set**: 12 NEW fixtures designed to attack specific axes of graph capability:

- **Graph-strength probes** (4): tasks where graph should clearly beat grep — multi-hop callers, transitive deps, re-export tracing, blanket/feature-gated impl enumeration.
- **Precision-gap probes** (4): tasks designed to expose the signature-vs-type-resolution gap that [`../../../docs/design/knowledge-graph/2_design.md`](../../../docs/design/knowledge-graph/2_design.md) §"Reference resolution" is honest about.
- **Payload-volume probes** (2): tasks that stress `pack` and reveal the firehose mode v3's `impact-tool-context-struct-literals` (12.43× on codex) exposed under controlled conditions.
- **Selector-ambiguity probes** (2): tasks whose natural query maps to multiple graph tools (search vs refs vs callers), testing whether agents pick the right tool given multiple plausible options.

No v1/v2/v3 fixtures carry into v4. Past data is sufficient on those.

**Non-experimental harness extensions** land alongside the fixture set — these are infrastructure additions required for the pre-registered report shape, not experimental variables:

- **Structured-output oracle.** A new oracle mode that parses agent output as `{"answer": [...], "excluded": [...]}` and grades `answer` against ground truth as a set. Existing oracle modes (grep, cmd, judge) remain available; structured grading is the v4 default. See §"Structured oracle format".
- **Failure-classification column in `aggregate.py`.** Adds per-run categorisation of failed graph calls (`schema-coercion`, `args-out-of-range`, `server-error`) and per-run failure-mode classification (`schema-coercion`, `payload-firehose`, `wrong-tool`, `oracle-artifact`, `design-defect`). Backward-compatible: v1/v2/v3 records read identically; v4 records gain new columns.
- **Per-tool token telemetry.** `aggregate.py` gains per-tool median/p90 output tokens by parsing tool-call result payloads from transcripts. Currently the aggregator counts invocations but not output volume per call.

These extensions are pre-registered here so the v4 freeze is reproducible from this method statement. They land in the same patch series as the fixture YAMLs.

## Pre-registered report shape

`RESULTS.md` will report (in this order):

1. **Per-fixture × per-arm × per-provider** primary table: pass rate (3 seeds), median total tokens, p90 total tokens, graph-call rate, failed-graph-call count.

2. **Per-category aggregate**: graph-only vs no-graph cost ratio (mean and worst-case), pass-rate delta per provider, hybrid graph-call rate (where applicable). Per-cell ratios are the load-bearing measurement; cross-cell aggregate medians may also be reported but are explicitly secondary (per the v3 lesson, see [`5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md) §"Methodological postscript").

3. **Per-tool diagnostic** for each of 8 `orbit_graph_*` tools: total invocations across the round, success rate, median output tokens per call, failure-mode breakdown.

4. **Failure taxonomy**: every failed run classified into one of:
   - `schema-coercion` — agent passed wrong arg shape (e.g. `include: "x"` vs `include: ["x"]`)
   - `payload-firehose` — run cost > 5× same fixture's no-graph median
   - `wrong-tool` — agent picked structurally inappropriate graph tool
   - `oracle-artifact` — answer semantically correct but oracle rejected (manual audit per case)
   - `design-defect` — hybrid run passed without invoking graph (fixture not actually graph-shaped)

5. **Production vs synthetic split**: every aggregate number reported twice — once for production-grounded fixtures, once for synthetic-island fixtures. Production numbers are the load-bearing measurement of day-to-day graph value; synthetic numbers measure mechanical capability.

6. **Standout fixtures**: top-3 wins (graph beats no-graph by ≥ 30% pass rate or ≥ 30% cost reduction), top-3 losses (graph worse by similar margin).

## Scope

- **Providers:** Claude (`claude-sonnet-4-6`) + Codex (`gpt-5.3-codex`)
- **Arms:** `no-graph`, `graph-only`, `hybrid`
- **Fixtures:** 12 NEW (7 production, 5 synthetic)
- **Seeds:** 3 per (provider × arm × fixture) cell
- **Hybrid scope:** runs only on the 8 graph-strength + precision-gap fixtures. Selector-ambiguity and payload-volume fixtures are graph-only diagnostic targets; running hybrid on them would muddy the selection signal without adding capability information.
- **Total runs:**
  - no-graph: 12 × 2 providers × 3 seeds = 72
  - graph-only: 12 × 2 providers × 3 seeds = 72
  - hybrid: 8 × 2 providers × 3 seeds = 48
  - **Total: 192 cells**
- **Sweep seed:** _TBD — populated at freeze from `runs/_sweeps/<provider>/<sweep_id>/order.json`._
- **Sweep date:** _TBD — populated at freeze._

## Fixture inventory

Each fixture has a YAML at `tasks/<fixture-id>.yaml` with prompt, deny-list, oracle, and ground truth. Synthetic fixtures additionally include `synthetic: true` and a `synthetic_code_path` pointing into `_fixture_code/`.

### Graph-strength (4)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `callers-2hop-graphbenchpolicy` | synthetic | ✓ | Find functions that transitively call `GraphBenchPolicy::resolve` via 2 hops, excluding direct callers. |
| `deps-downstream-orbit-knowledge` | production | ✓ | List crates that transitively depend on `orbit-knowledge` through 2 levels. |
| `reverse-export-orbit-error` | production | ✓ | List every module that re-exports `orbit_common::OrbitError` (or its production-equivalent re-export chain). |
| `implementors-auditsink-with-blanket` | synthetic | ✓ | Enumerate impls of `AuditSink` including a blanket impl `impl<T: Sink> AuditSink for Wrapper<T>` and a feature-gated impl. |

### Precision-gap (4)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `construct-vs-match-loopaudit-distinct` | synthetic | ✓ | Find files that CONSTRUCT `LoopAuditEvent::ToolCallResult{...}` via builder helper, nested constructor, or imported variant. Exclude pure pattern-match files. Distinct from v2's variant by adding the helper / nested / import patterns. |
| `function-as-value-vs-direct-call` | production | ✓ | Find sites where a real production function (target validated during authoring; candidates: `OrbitError::from`, conversion functions used in `.map_err(...)` chains) is passed as a value/closure callback, excluding sites that call it directly. |
| `generic-dispatch-concrete-impl` | synthetic | ✓ | At call-site `f::<ConcreteT>()`, identify which impl of `Trait::f` actually runs. |
| `macro-expanded-callers` | synthetic | ✓ | Find call-sites of `Default::default()` on a struct that derives `Default`. **Expected-fail sentinel** — graph parser does not expand derive macros; treat as known-loss baseline. |

### Payload-volume (2)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `impl-divergence-trait-method` | production | ✗ | Compare implementations of a real trait method across all impls. Real target validated during YAML authoring (candidates: `AuditSink::record`, `Backend::apply` if present, or another trait with 4–6 impls of bounded body size). |
| `const-value-extraction` | production | ✗ | List all `const DEFAULT_*` values across `orbit-common` with declared values. |

### Selector-ambiguity (2)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `references-vs-callers-tool-registry-register` | production | ✗ | "Where is `ToolRegistry::register` invoked from?" Tests refs vs callers vs search choice. |
| `module-surface-orbit-mcp` | production | ✗ | "What's the public surface of `orbit-mcp`?" Tests overview vs search-with-path vs show choice. |

## Synthetic-fixture indexer setup

Synthetic fixture code lives at `_fixture_code/`. The current `.orbitignore` excludes `benchmarks/**`, so `_fixture_code/` won't be indexed by default. v4 adds narrow negations to `.orbitignore`:

```
!benchmarks/
!benchmarks/graph/
!benchmarks/graph/v4/
!benchmarks/graph/v4/_fixture_code/
!benchmarks/graph/v4/_fixture_code/**
```

`benchmarks/graph/v4/runs/`, `benchmarks/graph/v4/tasks/`, and `benchmarks/graph/v4/_sweeps/` remain excluded so YAML and run artifacts don't pollute graph search.

Synthetic-fixture prompts explicitly scope the ask: *"in `benchmarks/graph/v4/_fixture_code/`"* rather than *"in Orbit production code."* Production-fixture prompts use *"in Orbit production code"* or name a specific crate. This separation keeps the benchmark honest: synthetic fixtures test graph mechanics; production fixtures test product utility.

## Structured oracle format

All v4 fixtures use a structured-output oracle:

```json
{
  "answer": ["<item>", "<item>", ...],
  "excluded": ["<item>", "<item>", ...]
}
```

The grader checks `answer` against ground truth as a set; `excluded` is informational and helps surface the agent's reasoning (and supports the `oracle-artifact` failure-taxonomy class). Item shape is fixture-specific (file path, symbol name, name+value pair, etc.) — declared in the YAML's `oracle.item_kind` field.

This replaces v3's substring-grep oracle, which produced false rejections on semantically-correct answers (notably v3 runs that mentioned excluded paths *as excluded* but failed substring matching). Substring grading is retained as a fallback only for trivial locate-style fixtures where the answer is a single short string.

## Failure-mode tracking

`aggregate.py` will gain a v4-specific column for failed graph calls. Failures are classified by reason:

- `schema-coercion` — `include: "x"` instead of `include: ["x"]`, etc.
- `args-out-of-range` — invalid symbol/path that the schema accepts but the server rejects
- `server-error` — index miss, parse error, internal panic

Successful retries after a failure are counted separately from un-recovered failures. This addresses the v3 observation that schema-coercion failures (e.g. agents passing `include` as a string) inflate cost without showing up in the pass/fail column.

## Known caveats (pre-run)

1. **Synthetic fixtures measure mechanical capability, not product utility.** Reported separately to avoid laundering capability numbers into product claims. The 7 production fixtures carry the day-to-day-utility diagnostic; the 5 synthetic fixtures carry the controlled-input mechanical-capability diagnostic. Decisions that affect shipped tools should weight production numbers; decisions about parser/index improvements may weight synthetic numbers.

2. **Hybrid scope is reduced (8 of 12 fixtures).** Selector-ambiguity and payload-volume fixtures probe graph internals, not selection. Adding hybrid on those four cells would burn 24 seeds for no incremental insight.

3. **`macro-expanded-callers` is an expected-fail sentinel.** Both providers are expected to fail this in graph-only because the graph parser does not expand derive macros. Reporting it as a "failure" in the pass-rate column is misleading; treat it as a known-loss baseline against which future macro-expansion work can be measured.

4. **No keep/cull threshold.** v4 produces structured findings, not a decision. Decisions land later as ADRs in `4_decisions.md` if v4's findings warrant.

5. **Per-cell pre-registration.** All comparisons are per-cell as in v3's strict reading (see [`5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md) §"Methodological postscript"). Aggregate medians may also be reported but are explicitly secondary.

6. **Hybrid prompts are neutral.** Same phrasing as graph-only and no-graph. A "graph-preferred" hybrid arm is rejected: it presupposes selection rather than measuring it. v4 measures organic selection on grep-impossible fixtures (a question v3's grep-solvable fixtures could not ask).

7. **Codex cost is still reported as $0.** The Codex CLI does not emit billing; USD figures remain Claude-only.

## Reproduction

v4 uses the shared harness at `benchmarks/graph/scripts/` with `GRAPH_VERSION=v4`. To run a single cell:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/run.py \
  --provider codex --arm hybrid --task callers-2hop-graphbenchpolicy --seed 1
```

To regenerate tables from run records:

```bash
make -C benchmarks graph-aggregate GRAPH_VERSION=v4
```

Or directly:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v4/runs \
  --tasks benchmarks/graph/v4/tasks
```

## Task References

_TBD — task IDs created during fixture authoring._

## Explicitly not in v4

- **Tool description rewrites.** Held constant from v3 so the measured signal is fixture-driven.
- **Pointer-only graph reads ([T20260423-0607]).** v4 measures payload volume on the existing payload shape; pointer-only is a separate experiment downstream of v4 findings.
- **Hybrid steering.** Hybrid prompts use the same neutral phrasing as graph-only and no-graph. (See caveat #6.)
- **Decision criteria.** v4 is diagnostic. No automatic keep/cull/redesign action triggers from v4 alone.
- **Carry-over fixtures.** No v1/v2/v3 fixture is reused. v4 measures NEW capability axes; old fixtures already produced settled data.

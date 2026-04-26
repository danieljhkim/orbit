# v4 — Method & Caveats

This file documents how the `graph_v4` sweep was planned and conducted. The frozen report is [`RESULTS.md`](./RESULTS.md). Conventions governing this document's shape are in `benchmarks/CONVENTIONS.md`.

v4 is explicitly scoped as a **diagnostic round**, not a keep/cull round. v3 settled retention of the agent-facing `orbit_graph_*` MCP surface (see [`../../../docs/design/knowledge-graph/5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md) §"Disposition"). v4 maps where the surface helps, where it hurts, and how it fails — with measured targets for future tool-shaping work (payload trimming, ref-kind precision, schema ergonomics).

## Harness SHAs at freeze

Codex pre-fix: `b0ce189e7053409c8754865bd154cd20e1de66a6`.
Claude run and Codex graph-only post-fix rerun: `56a9c07b64479360f9a64ca94b40721f76226014` (contains both `T20260425-0729` and `T20260425-0739`).

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

These extensions were pre-registered here so the v4 freeze is reproducible from this method statement. They landed in the same patch series as the fixture YAMLs.

## Pre-registered report shape

`RESULTS.md` reports (in this order):

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
- **Planned total runs:**
  - no-graph: 12 × 2 providers × 3 seeds = 72
  - graph-only: 12 × 2 providers × 3 seeds = 72
  - hybrid: 8 × 2 providers × 3 seeds = 48
  - **Total: 192 cells**
- **Post-fix rerun:** Codex graph-only was rerun after `T20260425-0729` and `T20260425-0739`, adding 12 × 3 = 36 cells for 228 recorded cells total. See [`RESULTS.md`](./RESULTS.md) for pre-fix/post-fix handling.
- **Sweep IDs / seeds:** Codex `20260424-230632-f36f84` / `928176111` (`no-graph`, pre-fix `graph-only`), Codex `20260425-001959-842b8c` / `583229300` (`hybrid`), Codex `20260425-115117-21a938` / `142643867` (post-fix `graph-only`), Claude `20260425-013339-cfac6a` / `152346771` (`no-graph`, `graph-only`), Claude `20260425-012511-8881cb` / `88160319` (`hybrid`).
- **Sweep dates:** 2026-04-24 to 2026-04-25.

## Fixture inventory

Each fixture has a YAML at `tasks/<fixture-id>.yaml` with prompt, deny-list, oracle, and ground truth. Synthetic fixtures additionally include `synthetic: true` and a `synthetic_code_path` pointing into `_fixture_code/`.

### Graph-strength (4)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `callers-2hop-graphbenchpolicy` | synthetic | ✓ | Find functions that transitively call `GraphBenchPolicy::lookup_rule` via 2 hops, excluding direct callers. Method renamed from `resolve` to avoid name collision with production `*::resolve` methods (verified empirically; see §"Synthetic-vs-production name isolation"). |
| `deps-downstream-orbit-knowledge` | production | ✓ | List crates that transitively depend on `orbit-knowledge` through 2 levels. |
| `reverse-export-orbit-error` | production | ✓ | List every module that re-exports `orbit_common::OrbitError` (or its production-equivalent re-export chain). |
| `implementors-benchsink-with-blanket` | synthetic | ✓ | Enumerate impls of synthetic trait `BenchAuditSink` (method: `record`, distinct from production `AuditSink::emit`) including a blanket impl `impl<T: Sink> BenchAuditSink for Wrapper<T>` and a feature-gated impl. |

### Precision-gap (4)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `construct-vs-match-benchevent-distinct` | synthetic | ✓ | Find files that CONSTRUCT synthetic enum `BenchAuditEvent::CallReturned{...}` (variant name distinct from production `LoopAuditEvent::ToolCallResult`) via builder helper, nested constructor, or imported variant. Exclude pure pattern-match files. |
| `function-as-value-vs-direct-call` | production | ✓ | Find sites where `RefName::new` is passed as a value (e.g. `.map(RefName::new)`), excluding sites that call `RefName::new(...)` directly. Production has 3 as-value sites and ~22 direct calls — bounded ground truth. |
| `generic-dispatch-concrete-impl` | synthetic | ✓ | At call-site `f::<ConcreteT>()`, identify which impl of `Trait::f` actually runs. |
| `macro-expanded-callers` | synthetic | ✓ | Find source-visible call-sites of `Default::default()` on a struct that derives `Default`; measures scoped call-site discovery and over-inclusion risk around derive-provided methods. |

### Payload-volume (2)

| id | mode | hybrid | one-line purpose |
|---|---|:---:|---|
| `impl-divergence-trait-method` | production | ✗ | Compare implementations of `AuditSink::emit` across all 4 production impls (`NullSink`, `InMemorySink`, `JsonlFileSink`, `EnforcedAuditSink`). Body sizes range 1–50 lines; the divergence between trivial impls and the policy-mirroring `EnforcedAuditSink` body is the load-bearing answer. |
| `const-value-extraction` | production | ✗ | List all `pub const` declarations (not `pub const fn`) in `orbit-common/src/types/` with their declared values. ~7 declarations: `AUDIT_ENVELOPE_SCHEMA_VERSION`, `ACTIVITY_REF_PREFIX`, `V2_TOOL_WILDCARD_ROOTS`, `DEFAULT_POLICY_NAME`, `UNRESTRICTED_FS_PROFILE`, `EXECUTOR_RESOURCE_SCHEMA_VERSION`, `POLICY_RESOURCE_SCHEMA_VERSION`. |

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

**Synthetic-vs-production name isolation.** Synthetic fixtures use distinct symbol names — including **distinct method and variant names**, not just distinct trait/struct/enum names. This is forced by graph's `callers` and `refs` semantics, which are name-based BFS (not type-resolved); a synthetic `BenchAuditSink::emit` would pollute callers queries on production `AuditSink::emit` and vice versa. Verified empirically: `GraphBenchPolicy::resolve` (synthetic) initially returned 59 callers across both `_fixture_code/` and production crates; renaming the method to `lookup_rule` cut the result to 6 clean synthetic-only callers.

Naming convention applied to v4 synthetic targets:
- `GraphBenchPolicy::lookup_rule` (not `::resolve` — collides with production `*::resolve`)
- `BenchAuditSink::record` (not `::emit` — collides with production `AuditSink::emit`)
- `BenchAuditEvent::CallReturned` (not `::ToolCallResult` — collides with production `LoopAuditEvent::ToolCallResult`)

The `macro-expanded-callers` fixture is an exception: its target is `Default::default()`, a stdlib trait method with thousands of production callers. That fixture's oracle must explicitly scope to `_fixture_code/` paths.

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

`aggregate.py` gained a v4-specific column for failed graph calls. Failures are classified by reason:

- `schema-coercion` — `include: "x"` instead of `include: ["x"]`, etc.
- `args-out-of-range` — invalid symbol/path that the schema accepts but the server rejects
- `server-error` — index miss, parse error, internal panic

Successful retries after a failure are counted separately from un-recovered failures. This addresses the v3 observation that schema-coercion failures (e.g. agents passing `include` as a string) inflate cost without showing up in the pass/fail column.

## Known caveats (pre-run)

1. **Synthetic fixtures measure mechanical capability, not product utility.** Reported separately to avoid laundering capability numbers into product claims. The 7 production fixtures carry the day-to-day-utility diagnostic; the 5 synthetic fixtures carry the controlled-input mechanical-capability diagnostic. Decisions that affect shipped tools should weight production numbers; decisions about parser/index improvements may weight synthetic numbers.

2. **Hybrid scope is reduced (8 of 12 fixtures).** Selector-ambiguity and payload-volume fixtures probe graph internals, not selection. Adding hybrid on those four cells would burn 24 seeds for no incremental insight.

3. **`macro-expanded-callers` is not a macro-expanded-impl oracle.** It asks for source-visible call-sites of a derive-provided method, not the generated `Default` impl body. Count it normally in pass-rate tables; macro-expanded impl indexing should be tested by a separate future fixture if needed.

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

- **T20260425-0729** — scalar string-list coercion across graph inputs, discovered by v4 pre-fix runs.
- **T20260425-0739** — pub-use re-export indexing in file exports metadata, discovered by v4 pre-fix runs.
- **T20260425-2140** — `source_regex` follow-up motivated by v4 payload/call-count findings and validated in v5.

## Explicitly not in v4

- **Tool description rewrites.** Held constant from v3 so the measured signal is fixture-driven.
- **Pointer-only graph reads ([T20260423-0607]).** v4 measures payload volume on the existing payload shape; pointer-only is a separate experiment downstream of v4 findings.
- **Hybrid steering.** Hybrid prompts use the same neutral phrasing as graph-only and no-graph. (See caveat #6.)
- **Decision criteria.** v4 is diagnostic. No automatic keep/cull/redesign action triggers from v4 alone.
- **Carry-over fixtures.** No v1/v2/v3 fixture is reused. v4 measures NEW capability axes; old fixtures already produced settled data.

# Graph Token-Usage Benchmark — Series Wrap-Up (v1-v5)

**Status:** Closed.
**Series dates:** roughly Q1-Q2 2026.
**Scope:** 5 rounds measuring whether Orbit's structured graph tools (exposed via MCP) save tokens / improve correctness over the agent's default shell-based code navigation, on Claude (Sonnet 4.6 + Haiku 4.5) and Codex (GPT-5.3-Codex).

This document is the cross-round synthesis. Per-round artifacts live in `v1/`, `v2/`, `v3/`, `v4/`, `v5/`.

---

## What this was

The original question: *when does the per-call token overhead of an MCP-exposed structured tool pay for itself, compared to the same agent navigating the codebase via its native shell tool (`exec_command` for Codex, `Bash` for Claude)?*

Each round used three arms — `no-graph` (shell only), `graph-only` (MCP graph tools only, no shell), and `hybrid` (both available, agent chooses) — across a fixture set probing distinct query shapes (callers, implementors, re-exports, constants, interfaces, etc.).

---

## What we learned

### 1. MCP overhead is bimodal

The dominant cross-round finding. Structured tools pay for themselves in two distinct cases, and are dead weight everywhere else:

- **The agent's no-graph baseline is wasteful.** Codex without graph spends ~16,400 tokens to enumerate every transitive downstream of a target crate (it walks the dep tree by reading source). With `orbit.graph.deps`, ~2,250 — `0.14×` the cost at full accuracy. Whenever the agent is verbose enough that a structured query collapses the work, MCP wins.
- **Source search has miss-risk.** Claude with `grep`+`Read` on `const-value-extraction` (find every `pub const` in a module) passed 1 of 3 attempts; on the other two it silently missed `V2_TOOL_WILDCARD_ROOTS`. With `orbit.graph.search`, 3 of 3 — at `9.71×` the no-graph token cost. The graph enumerates structurally; grep doesn't. Whenever silent miss is unacceptable, MCP wins on reliability even if it costs more.

Outside those regimes, MCP is overhead. On the median Claude task in v4, graph-only used `3.27×` the tokens of no-graph and arrived at the same answer.

This bimodal framing is the practical takeaway for anyone deciding whether to ship an MCP tool surface to coding agents.

### 2. Providers don't reach for graph the same way

Same prompts, same tools, same fixtures — Codex reaches for the structured tool, Claude mostly doesn't.

| | hybrid graph-call rate (v4) | no-graph median tokens (v4) |
|---|---:|---:|
| Codex | 11/24 | 11,446 |
| Claude | 3/24 | 713 |

Claude's no-graph baseline is roughly **16× tighter** than Codex's. This makes ratio-based readings dangerous: Claude's "graph-only is 3.27× no-graph" looks bad, but the denominator is 713 tokens. Claude graph-only is *cheaper than Codex no-graph* on every fixture in absolute terms.

The implication for the bimodal framing above: which case applies depends not just on the question shape but on the agent's baseline frugality. A 16,400-token Codex query has lots of room for a structured tool to compress. A 713-token Claude query has almost none. So the same MCP tool can be a clear win for one provider and dead weight for another.

### 3. p90, not median, is the real cost driver

Aggregate medians hide both wins and losses. The long tail is where MCP cost actually hurts — and it's almost entirely payload-firehose, not call count.

In v4 post-fix Codex graph-only:
- `callers` p90 response = **319,274 chars**
- `refs` p90 response = **116,359 chars**
- `overview` p90 response = **128,204 chars**

The expensive-but-correct fixtures (`const-value-extraction` 9.71×, `reverse-export-orbit-error` 6.73× / 8.84×) were almost all driven by one or two firehose-class calls per seed, not by call count. Future tool work should focus on payload shaping for high-cardinality responses (pointer-by-default, lazy hydration) rather than adding new query primitives.

### 4. Skills > tool descriptions for agent affordance

v5's source_regex experiment revealed the boundary clearly. The feature shipped with a strong tool description but agents over-iterated anyway (15-19 `source_regex` calls per seed on `reverse-export`, varying the prefix per crate, when one broad prefix would return the answer set in a single call).

Reason: tool descriptions are *parameter contracts* — agents skim them for arg shapes, not strategy. Workflow guidance ("ONE broad prefix, do NOT iterate") belongs in *skills*, which are loaded on-demand and have room for decision trees, anti-patterns, and worked examples.

Both Claude and Codex auto-discover skills (Claude from `.claude/skills/`, Codex from `.agents/skills/`), so a single source-of-truth file at `crates/orbit-core/assets/skills/orbit-graph/SKILL.md` reaches both providers. The skill update predicting the affordance fix shipped in v5 but its empirical re-validation is deferred (the next run would have been v6).

The wider principle: **don't put strategy in tool descriptions.** Tool descriptions describe the tool. Skills describe usage.

### 5. The benchmark caught real bugs that synthetic testing wouldn't have

Two graph indexer defects were identified by empirical agent runs and would not have surfaced from unit tests or static analysis:

- **`T20260425-0739`** — the parser dropped `pub use` re-exports from file `exports` metadata. v4 graph-only Codex returned `[]` on `reverse-export-orbit-error` (0/3 pass at 5.59× tokens), spending tokens trying to find files the graph confidently said had no exports. After fix: 3/3 pass.
- **`T20260425-0729`** — `refs.include` and `pack.selectors` rejected scalar string inputs that agents naturally produce (e.g. `include: "code"` instead of `include: ["code"]`). Caused 26 of 28 pre-fix Codex graph-only failed graph calls. Pure ergonomics defect.

Both bugs were silent-wrong-answer or silent-friction class — invisible from inside the codebase, only visible when an agent's behavior surfaced them. The benchmark's value-add over conventional testing was specifically *empirical agent behavior under realistic prompts*.

### 6. Hybrid is the practical operating mode, but for asymmetric reasons

Across both providers in v4, hybrid passed every cell on the graph-eligible subset. But the *meaning* of that win differs:

- Codex hybrid (11/24 graph calls) — selective graph use; the "right tool for the question" pattern. Median 3,900 tokens vs no-graph 11,446 = real compression.
- Claude hybrid (3/24 graph calls) — almost all graph-avoidant. Hybrid's win for Claude is mostly "having grep available alongside the structured tool" — the structured tool sat unused on the table. Median 663 tokens vs no-graph 713 = essentially the same.

So hybrid is the safest configuration ("agent picks whatever works"), but it doesn't prove that the structured tool is doing meaningful work in every cell. For Claude specifically, hybrid is more about not regressing than about gaining anything from MCP.

---

## Tool-side changes that shipped during the series

| change | task | round triggered | mechanism |
|---|---|---|---|
| `pub use` re-export indexing in file `exports` metadata | `T20260425-0739` | v4 | Parser bug fix |
| Scalar-as-array coercion across graph tool inputs | `T20260425-0729` | v4 | Tool ergonomics |
| `source_regex` filter on `orbit.graph.search` | `T20260425-2140` | v4 → v5 | New tool capability |
| `orbit-graph` skill update with anti-iteration guidance | (skill commit `1d306f03`) | v5 | Affordance — shipped but not re-validated |

The first three were validated empirically; the skill update is a predicted remediation pending v6 (which we're not running).

---

## Per-round arc

| round | scope | core question | key learning |
|---|---|---|---|
| **v1** | initial baseline, single provider | does graph save tokens at all? | Indexer pollution from benchmark transcript files; production-name collisions invalidate structural queries. |
| **v2** | extended fixtures | does the graph index correctly? | Type-resolution gaps on common patterns; oracle artifacts dominated the failure taxonomy. |
| **v3** | calibrated cost, both providers | does graph beat no-graph on cost? | Hybrid emerges as the practical operating mode. Per-cell vs aggregate threshold reading disagreed; v3 was a published null result that pre-registered v4 methodology. |
| **v4** | diagnostic, 192 cells, full provider × arm × seed | what's the failure mode taxonomy? | Two graph indexer bugs identified and fixed (T0729, T0739). Call-count waste class identified. Pre-fix vs post-fix ratios honest only when both arms are correct. |
| **v5** | feature-validation, 9 cells, Codex graph-only | did `source_regex` deliver? | Feature works (60-63% token reduction on fitting fixtures) but agents over-iterate. Skill update is the predicted affordance fix; the right place for workflow guidance is skills, not tool descriptions. |

---

## Why we stopped here

The remaining bottleneck shifted from *tool design* to *agent discipline*.

v1-v4 surfaced graph-tool defects. We fixed them. v4 motivated `source_regex`; v5 validated it ships and works, and identified that agents need skill-level guidance to use it well. The skill update closes that loop.

What v6 would have measured (and why we didn't run it):
- A re-run of v5's 3 fixtures with the updated skill, to verify the call-count ceiling is hit.
- A full sweep matching v4's 192-cell scope to confirm no regressions on other fixtures.

Both are reasonable but neither would change the fundamental conclusions above. The remaining product questions — payload shaping for `callers`/`refs`/`overview` p90, kind-filter affordances for the residual structural queries `source_regex` doesn't fit — are independent tracks that would benefit more from focused fixture probes than from another full sweep. If we run another round, it should be designed around a specific tool-side change worth validating, not a re-measurement of what we already know.

---

## Reading guide

- [`v1/RESULTS.md`](v1/RESULTS.md), [`v2/RESULTS.md`](v2/RESULTS.md) — early rounds; mostly indexer/oracle hygiene.
- [`v3/RESULTS.md`](v3/RESULTS.md) — the published null result. The pre-fix vs aggregate threshold disagreement is documented in [`docs/design/knowledge-graph/5_null_result.md`](../../docs/design/knowledge-graph/5_null_result.md).
- [`v4/METHOD.md`](v4/METHOD.md), [`v4/RESULTS.md`](v4/RESULTS.md) — diagnostic round, 192 cells, full Codex × Claude × arm. The most data-rich round; the source for findings 1-3 above.
- [`v5/RESULTS.md`](v5/RESULTS.md) — feature-validation closer; 9 cells. The source for finding 4.

For the methodological narrative (why per-cell thresholds, why synthetic name isolation, why structured-oracle JSON answer-shape) see `v4/METHOD.md` — it's the most-developed pre-registered methodology in the series.

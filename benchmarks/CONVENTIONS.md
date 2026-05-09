# Benchmark Conventions

Layout, versioning, and report shape for benchmarks under `benchmarks/`. Two kinds:

- **agent** — measure agent behavior under a tool surface (token budget, pass rate, tool utilization). LLM in the loop. e.g. `graph/`.
- **perf** — measure system properties directly (latency, durability, resource cost). No agent. e.g. `graph-latency/`, `identity-key/`.

Common rules apply to both. `RESULTS.md` and `METHOD.md` schemas differ by kind.

---

## Common rules

### Benchmark kind

Each benchmark's top-level `README.md` MUST have `kind: agent` or `kind: perf` as the first body line under the H1 — plain text, grep-able, not YAML frontmatter. The declared kind selects the report schema for every `vN/` round. A benchmark cannot mix kinds; if the question changes shape, cut a new benchmark directory.

### Directory layout

```
benchmarks/<name>/
├── README.md          # shared overview; kind: ...; versions table
├── scripts/           # SHARED harness, expected to stay back-compat with frozen runs/
├── mcp.json           # shared transport (agent kind, when applicable)
├── v1/                # FROZEN — immutable
│   ├── README.md      # frozen banner
│   ├── METHOD.md
│   ├── RESULTS.md
│   ├── ISSUES.md      # optional
│   ├── tasks/         # fixtures (agent) or query-shape defs (perf)
│   └── runs/          # per-cell records + _sweeps/<id>/
└── v2/                # LIVING — in-progress
    ├── README.md      # living banner
    ├── tasks/         # mutable
    └── runs/          # gitignored staging
```

Per-round `scripts/` and `mcp.json` exist *only* if a round broke back-compat with the shared harness at freeze time; otherwise rounds rely on `../scripts/`. Frozen `vN/` is immutable; the shared `scripts/` + living `vN/` represent "what the next round will use."

### When to cut a new version

A version captures a sweep campaign — report plus all inputs that produced it. Cut a new version when any of these change vs the prior frozen version:

1. Fixture set (tasks added/removed/edited; corpus list or query-shape defs changed for perf).
2. Harness code that affects measurement (scoring, normalization, classifier; timing/sampling, record schema, host disclosure for perf).
3. Model/provider lineup (agent), or system-under-test version pin (perf).
4. Interpretive frame (e.g. adding a tool-utilization audit or a budget column).

Do NOT cut for: stochastic re-runs, typo fixes (use `CORRECTIONS.md`), or measurement-neutral refactors.

### Freezing a version

1. Sweep complete, `RESULTS.md` written.
2. Drop the `runs/.gitignore` so records commit.
3. Author `METHOD.md` per kind-specific schema below.
4. Optionally prune failed staging sweeps under `runs/_sweeps/`.
5. Replace the LIVING banner in `vN/README.md` with a frozen banner.
6. Back-compat check: shared `scripts/` still reproduces `vN/runs/` numbers. If not, snapshot `scripts/` (and `mcp.json` if needed) into `vN/` and note in METHOD.md §Delta.
7. Scaffold `v(N+1)/`: copy `vN/tasks/`, empty `runs/` with `.gitignore`, LIVING README.
8. Bump default `<NAME>_VERSION` in `Makefile`.
9. Update versions table in `benchmarks/<name>/README.md`.

Freeze lands as one commit, separate from v(N+1) development.

### Runs and git

- **Living `runs/`** — `.gitignore` excludes everything except itself. Staging only.
- **Frozen `runs/`** — committed. Records and any transcripts are the evidence behind `RESULTS.md`.

Before committing a new frozen version, scan transcripts for secrets (API key prefixes, bearer tokens). Matches block the commit. Env-var *names* are acceptable; absolute paths leaking a username are acceptable if the same name appears in `git log`.

### Immutability

Once `vN/RESULTS.md` is committed, the directory is read-only by convention.

- Factual correction → append to `vN/CORRECTIONS.md`. Don't mutate report/method/runs.
- Reinterpretation with same evidence → new METHOD.md §Delta in v(N+1), or a `COMPARE-vN-vs-vM.md` at the shared level.
- New evidence → cut a new version.

If a frozen snapshot must be mutated for a non-fitting reason (e.g. accidentally committed secrets), say so explicitly in the commit message.

### Tooling and references

`benchmarks/Makefile` accepts a per-benchmark version variable (`GRAPH_VERSION`, `GRAPH_LATENCY_VERSION`, …) that swaps `vN/` in script paths. Unset defaults to the living harness.

In docs, PRs, and commits: cite `benchmarks/<name>/vN/RESULTS.md` plus the SHA at which it landed. Never write "the benchmark results" without a version — that refers to mutable state and rots. PRs claiming a perf or accuracy effect must either (a) cite a frozen version, or (b) include a new frozen version produced by the PR.

---

## RESULTS.md (agent benchmarks)

Required sections, in order:

1. **Frontmatter** — task ID, sweep date, seed(s), scope (providers × arms × fixtures × seeds), fixture list. One paragraph.
2. **Headline** — 3–6 bullets. Lead with the most load-bearing finding. If a naive read of the tables would mislead, say so here.
3. **Tool-utilization audit** — per-arm counts of which tools agents actually called. A cost table without utilization is incomplete. Omit only if the benchmark structurally cannot vary tool surface.
4. **Primary table** — provider × arm × task_class. Columns: `runs`, `pass_rate`, `median_total_tokens`, `p90_total_tokens`, `tokens_per_success`. From `scripts/aggregate.py`.
5. **Cost (USD)** — per-arm totals where billing is reported. Note explicitly when a `$0` is a CLI limitation, not free usage.
6. **Pass-rate breakdown** — every failed run by (provider, arm, fixture, seed) with rejection reason.
7. **Re-interpretation** (encouraged) — reconcile raw tables vs utilization audit when they disagree.
8. **Hypothesis reconciliation** — pre-sweep hypotheses graded ✅/❌/⏸. If none were filed, say so.
9. **Recommendations** — separate "change in product" from "change in next sweep."
10. **Methodology notes** — token-accounting convention, caveats, reproduction command, ad-hoc analyses outside `aggregate.py`.

## METHOD.md (agent benchmarks)

1. Harness git SHA at freeze time.
2. Delta vs v(N-1) — fixture/harness/model/scope diff. v1: write "v1 is the first frozen round; no prior version to diff."
3. Fixture list with one-line purposes.
4. Known caveats.
5. Reproduction command.

---

## RESULTS.md (perf benchmarks)

No agent in the loop. No `pass_rate`, no `tokens_per_success`, no per-arm hypothesis grading. Required sections, in order:

1. **Frontmatter** — task ID, sweep date, seed(s), scope (corpora × tools/scenarios × phases × seeds), `orbit_sha`. One paragraph.
2. **Headline** — 3–6 bullets. Lead with what's over budget, where the regression or improvement lives.
3. **Primary table** — corpus × tool × phase. Columns: `runs`, `p50_ms`, `p90_ms`, `p99_ms`, `budget_ms`, `over_budget`. From `scripts/aggregate.py`. Non-latency perf benchmarks substitute their analogous primary observation table — but columns must expose distribution shape, not point estimates.
4. **Build-phase table** — for benchmarks with an indexer or other state-building step: cold full + warm incremental, per corpus tier. Columns: `corpus`, `phase`, `wall_ms` (p50/p90/p99), `rss_peak_mb`, `budget_ms`. Omit only if no build phase exists.
5. **Host/environment disclosure** — CPU, RAM, OS. State whether all primary-table rows came from one host or are aggregated (cross-host aggregation requires per-row host annotation).
6. **Delta vs v(N-1)** — absolute and percent change per row that exists in both versions. Lead with regressions; call out improvements explicitly. v1: "v1 is the first frozen round; no prior version to diff."
7. **Known caveats** — cold/warm cache, GC pauses, indexer parallelism — anything that shifts numbers if reproduced naively.
8. **Recommendations** — separate "change in product" from "change in next sweep."

## METHOD.md (perf benchmarks)

1. Harness git SHA at freeze time.
2. Delta vs v(N-1) — corpus/query-shape/harness/SUT-pin/scope diff. v1: "v1 is the first frozen round; no prior version to diff."
3. Corpus list with `<org>/<repo>@<sha>` rows, LOC tier, language, and fetch instructions (typically `~/.cache/orbit-bench/<corpus>` populated by `scripts/fetch.sh`).
4. Per-cell record schema — JSON shape of each `runs/*.json` field-by-field. For latency benchmarks: `corpus`, `tool`, `query_shape`, `phase`, `seed`, `wall_ms`, `rss_peak_mb`, `result_size_bytes`, `result_count`, `host` (`cpu`, `ram_gb`, `os`), `orbit_sha`, `corpus_sha`. Other perf benchmarks substitute their own field set.
5. Host disclosure rules — what metadata the harness records, and the policy on cross-host comparisons. State explicitly whether `RESULTS.md` numbers are single-host or aggregated.
6. Reproduction command.

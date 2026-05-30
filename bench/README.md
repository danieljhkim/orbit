# Graph Benchmark Baselines

`bench/baselines.json` is the committed source of truth for the orbit-graph
performance gate described in
`docs/design/orbit-graph/specs/GRAPH_SPEC.md` section 12. CI compares a run
against these committed values, not against the previous merged run.

## Runner Profile

Baseline values are measured on the GitHub Actions runner profile from the
spec:

- Image: `ubuntu-24.04`
- OS release for the captured run: `Ubuntu 24.04.4 LTS`
- Logical cores: 4
- Memory: 15989 MiB

Local runs are useful for investigation, but local values are advisory only
and must not be committed as baseline updates.

## Capture Procedure

Capture a new baseline from a recorded GitHub Actions run and include the run
URL, job URL, and raw output in the Orbit task or PR notes before changing
`bench/baselines.json`.

For the initial v1 capture, the CI job built release binaries, ran
`target/release/examples/graph_build --workspace "$PWD"`, measured the
one-file incremental update with `orbit graph update`, measured query rows
through the `orbit.graph.*` tool surface, and recorded
`.orbit/knowledge/graph/graph_index.sqlite` size. The `impact depth=3` row uses
`orbit.graph.callers` at depth 3 as the v1 proxy because `orbit-knowledge` does
not expose an `impact` command.

P6.2 owns wiring the permanent `graph_bench.rs` CI gate. Once that lands, use
the gate artifact under `target/bench/` as the capture source instead of an
ad-hoc capture script.

## CI Regression Gate

The `Graph Bench / Graph Bench` PR job runs on the pinned
`ubuntu-24.04` runner profile from the spec. The workflow is path-filtered to
Rust source, Cargo, graph-bench, and workflow files so docs-only PRs do not
spend a benchmark runner.

The job builds the release benchmark binaries, runs the graph benchmark three
times, and writes `target/bench/results.json`. Each reported row stores the raw
samples and the median value used for the gate. The artifact is uploaded as
`graph-bench-results` on the PR's Actions page.

Gate decision for ORB-00321: v2 `orbit-graph` rows are gated against
`bench/baselines.json`; v1 `orbit-knowledge` rows from `graph_bench.rs` remain
informational in the artifact. GRAPH_SPEC section 12 sets budgets for the v2
implementation, while the v1 rows are retained during the transition so
reviewers can compare old and new behavior.

`bench/check_baseline.sh` compares every gated row in
`target/bench/results.json` against the committed baseline row with the same
ID. A row fails when its median value is more than 20 percent slower than the
baseline value.

## Updating Baselines

A PR that changes `bench/baselines.json` must have the
`bench-baseline-bump` label and a one-line justification in the PR body.
Use a line in this exact form so CI can verify it:

```text
bench-baseline-bump: <why this baseline change is intentional>
```

When the label and justification are present, CI still runs the benchmark and
uploads `target/bench/results.json`, but skips the regression check. Without
the label, or with the label but no justification line, the gate runs normally
or fails the PR-body validation.
Routine performance wins should bump the baseline down. Routine performance
drift should not bump the baseline; fix the regression instead.

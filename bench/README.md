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

## Updating Baselines

A PR that changes `bench/baselines.json` must have the
`bench-baseline-bump` label and a one-line justification in the PR body.
Routine performance wins should bump the baseline down. Routine performance
drift should not bump the baseline; fix the regression instead.

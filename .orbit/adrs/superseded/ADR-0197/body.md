## Context

ADR-0192 rolled back the orbit-graph v2 tool cutover to orbit-knowledge and, as one of its consequences, decided to **keep the `orbit-graph` crate and the equivalence harness in tree** to gate a future cutover. The equivalence harness is the `tools/graph-equiv` workspace binary (plus its frozen multi-language corpus) and the `bench/` benchmark-baseline scripts (`baselines.json`, `check_baseline.sh`, `run_graph_bench_ci.sh`, `equiv-waivers.md`). It dual-ran the v1/v2 backends over a frozen corpus and failed CI (`make ci-equiv`, the `graph-equiv` GitHub Actions job) on any diff outside documented tolerances.

In practice the v2 cutover is paused indefinitely (ADR-0192) and none is scheduled. The harness nonetheless carries standing cost: a Cargo workspace member, a dedicated CI job, a Makefile target, a `check-dependency-direction.sh` guardrail entry, and four documentation references — all maintaining a gate for a migration step that is not active.

## Decision

Remove the equivalence and benchmark harness from the tree: delete `tools/graph-equiv/` (and its frozen corpus) and `bench/`, and unwire them from the build — the Cargo workspace member, the `make ci-equiv` target, the `graph-equiv` CI job, and the dependency-direction guardrail allowlist entry.

This **amends ADR-0192**: it reverses *only* ADR-0192's "keep the equivalence harness in tree" consequence. ADR-0192's primary decision — the rollback of the v2 cutover and the gates required before any future cutover — remains fully in force, and the `orbit-graph` crate itself is **kept**. Only the equivalence/benchmark tooling is removed.

The v1↔v2 equivalence relation documented in GRAPH_SPEC still defines what a future cutover must satisfy; if a cutover is rescheduled, the harness is reintroduced fresh as part of that work rather than carried indefinitely as an inactive scaffold.

## Consequences

- The workspace drops one crate and the `graph-equiv` CI job; `make ci-equiv` no longer exists. CI and `cargo check --workspace` are unaffected otherwise.
- The documented equivalence relation (GRAPH_SPEC §migration) becomes plan-only: no in-tree binary enforces it until a cutover is rescheduled.
- ADR-0192's harness-retention consequence is amended by this ADR; its rollback decision and pre-cutover gates are unchanged.
- Cost: a future v2 cutover must rebuild the equivalence + benchmark harness — binary, frozen corpus, baselines, and CI wiring — from scratch rather than resuming an in-tree scaffold, and the existing frozen corpus and baseline history are lost.
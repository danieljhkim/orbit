# Graph Latency Benchmark v2

**Status: FROZEN** as of 2026-05-09. Records under `runs/`,
[`METHOD.md`](./METHOD.md), and [`RESULTS.md`](./RESULTS.md) are immutable per
[`../../CONVENTIONS.md`](../../CONVENTIONS.md) §Immutability. Factual
corrections go in `CORRECTIONS.md`; reinterpretation goes in v3 §Delta or a
shared compare doc.

- Method: [`METHOD.md`](./METHOD.md)
- Results: [`RESULTS.md`](./RESULTS.md)
- Run records: [`runs/`](./runs/)

## Headline

Single variable changed vs v1: orbit binary SHA. v2's `orbit_sha=f6097e0a` is
accurate (cargo-installed immediately before sweep); v1's recorded SHA was a
harness-checkout proxy and the actual v1 binary was a stale `orbit-cli v0.1.0`.
v1→v2 delta therefore measures "stale v0.1.0 → fresh v0.3.1 release-mode" and
is best read as "establishing the first reliable baseline" rather than a clean
code-change delta.

Two material regressions: Python `graph.refs` p50 +32%, Java build-incremental
p50 +21%. Most other cells drifted within ±10%. The "incremental slower than
cold" gap widened in all three languages. v1's structural failure pattern
reproduced exactly. See [`RESULTS.md`](./RESULTS.md).

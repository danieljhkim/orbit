# Graph Latency v2 Results

kind: perf-results
status: living
task: ORB-00380
date: 2026-06-13

## ORB-00380 Read-Path Measurement

| Tool | Indexed p50 | Legacy fallback p50 | Delta |
| --- | ---: | ---: | ---: |
| `orbit.graph.show` | 444.74 ms | 4,273.30 ms | 9.6x faster |
| `orbit.graph.refs` | 474.95 ms | 4,031.53 ms | 8.5x faster |
| `orbit.graph.callers` | 464.22 ms | 5,955.98 ms | 12.8x faster |

Raw timings, milliseconds:

| Tool | Indexed samples | Legacy fallback samples |
| --- | --- | --- |
| `show` | 461.3, 446.5, 460.8, 443.4, 444.5, 443.1, 444.9, 443.8, 445.7, 441.7 | 4309.5, 4294.7, 4332.6, 4262.4, 4263.4, 4274.1, 4272.5, 4317.5, 4269.2, 4269.7 |
| `refs` | 461.8, 452.8, 450.8, 561.4, 515.1, 479.9, 484.5, 464.2, 483.7, 470.0 | 4233.8, 4036.4, 4045.9, 3996.4, 4067.6, 4032.4, 4030.7, 4007.6, 3997.4, 3994.2 |
| `callers` | 492.0, 462.5, 465.6, 478.2, 466.3, 463.8, 464.2, 464.2, 463.7, 463.8 | 6083.3, 5970.4, 5953.6, 5946.4, 5948.7, 6060.6, 5958.4, 5933.6, 5944.2, 5958.4 |

## Interpretation

The indexed path still pays CLI process startup in this harness, so absolute
numbers are not the in-process lower bound. The delta is the useful signal:
the legacy fallback spends seconds hydrating 18k graph objects, plus source
blobs or tree-sitter parsing for refs/callers, while the indexed path performs
bounded SQLite lookups and materializes only result rows.

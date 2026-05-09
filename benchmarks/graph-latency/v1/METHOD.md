# Graph Latency Benchmark v1 Method

## Harness git SHA at freeze time

`<TBD-at-freeze>`. The harness skeleton lands with task `T20260509-63`; the
first real sweep and the SHA pin land with a follow-up task.

## Delta vs v0

v1 is the first frozen round; no prior version to diff.

## Corpus list

Three tiers across three languages — Python, Java, TypeScript. All corpora are
pinned at a specific upstream SHA and fetched into `~/.cache/orbit-bench/<corpus>`
by `scripts/fetch.sh`. Pins listed below are the v1 starting set; concrete commit
SHAs are resolved and locked in the first sweep (the placeholder `<sha>` markers
below become real 40-char hashes once the sweep runs and `git rev-parse HEAD` is
recorded).

The `large` tier targets ~700k–1M LOC, not the 2M+ enterprise regime. 2M+
mono-repos exist (cpython core, IntelliJ Community, full OpenStack) but are rare
in practice; sizing the tier at 700k–1M matches the regime most users actually
hit graph-latency walls in.

| Corpus name      | Language   | Tier   | Source                                              | LOC target  |
|------------------|------------|--------|-----------------------------------------------------|------------:|
| `python-small`   | Python     | small  | `pallets/flask@<sha>`                               |        ~10k |
| `python-medium`  | Python     | medium | `django/django@<sha>`                               |       ~280k |
| `python-large`   | Python     | large  | `home-assistant/core@<sha>`                         |       ~700k |
| `java-small`     | Java       | small  | `apache/commons-cli@<sha>`                          |        ~12k |
| `java-medium`    | Java       | medium | `google/guava@<sha>`                                |       ~150k |
| `java-large`     | Java       | large  | `apache/hadoop@<sha>`                               |        ~1M  |
| `ts-small`       | TypeScript | small  | `preactjs/preact@<sha>`                             |        ~10k |
| `ts-medium`      | TypeScript | medium | `vuejs/core@<sha>`                                  |       ~150k |
| `ts-large`       | TypeScript | large  | `angular/angular@<sha>`                             |       ~600k |

TypeScript is included because `orbit-knowledge` parses it as a first-class
language and TS exercises pathologies neither Python nor Java cover well —
barrel re-exports (`export * from './x'`), `import type` vs value imports,
and conditional types. The closed `graph/` series found a `pub use` re-export
parser bug in v4 (`T20260425-0739`); barrel files are the JS/TS analog and a
likely place for parser surprises.

The "v1 starting set, may revise on first sweep" caveat applies to every row:
if a candidate repo turns out to be unrepresentative, unfetchable, or
mis-sized, the first-sweep task substitutes a replacement and records the
substitution in `RESULTS.md` §Known caveats. `ts-large` (`angular/angular`,
~600k) sits slightly under the 700k–1M tier floor — it is the cleanest
canonical large-TS choice; the next-best fit (`vercel/next.js`, ~700k)
requires path-filtering during fetch and was deferred for now.

### Fetch instructions

```bash
# All corpora
make -C benchmarks graph-latency-fetch

# A single corpus
benchmarks/graph-latency/scripts/fetch.sh --version v1 --corpus python-medium
```

The fetch script is idempotent: existing checkouts at the expected SHA are
skipped. Total disk footprint is approximately a few GB across all six tiers.

## In-scope tools

All nine `orbit.graph.*` MCP tools, one cell per tool per corpus per phase:

- `orbit.graph.overview`
- `orbit.graph.search`
- `orbit.graph.callers`
- `orbit.graph.deps`
- `orbit.graph.refs`
- `orbit.graph.show`
- `orbit.graph.implementors`
- `orbit.graph.history`
- `orbit.graph.pack`

`graph.history` and `graph.pack` are included in v1 even though their typical
production usage differs from the others — establishing baselines on every
public surface keeps the matrix complete and lets later rounds drop cells if
the cost is uninteresting.

## Phases

Two measurement phases:

- **build-cold** — full index of the corpus from a clean cache. One observation per corpus per seed.
- **build-incremental** — incremental rebuild after a controlled mutation (rename one file, edit one symbol body, move one file). One observation per corpus per seed per mutation.
- **query** — N seeded calls of each tool against the built index. Distribution reported as p50/p90/p99 across seeds.

## Per-cell record schema

Each `runs/<corpus>/<tool>/<phase>/<seed>.json` record has exactly these fields:

| Field                | Type    | Notes                                                                 |
|----------------------|---------|-----------------------------------------------------------------------|
| `corpus`             | string  | corpus name from the table above (e.g. `python-medium`)               |
| `tool`               | string  | `graph.<name>` for query phase; `null` for build phases               |
| `query_shape`        | string  | id from `v1/tasks/<tool>.yaml`; `null` for build phases               |
| `phase`              | string  | one of `build-cold`, `build-incremental`, `query`                     |
| `seed`               | integer | 1-indexed seed for query selection                                    |
| `wall_ms`            | integer | wall-clock duration of the measured operation in milliseconds         |
| `rss_peak_mb`        | integer | peak resident set size during the operation, in MiB                   |
| `result_size_bytes`  | integer | size of the JSON tool result; `null` for build phases                 |
| `result_count`       | integer | top-level result count from the tool; `null` for build phases         |
| `host`               | object  | `{ "cpu": str, "ram_gb": int, "os": str }`                            |
| `orbit_sha`          | string  | 40-char git SHA of the orbit binary under measurement                 |
| `corpus_sha`         | string  | 40-char git SHA of the corpus checkout                                |

The aggregator reads only these fields. Adding new fields is allowed and
non-breaking; removing or renaming any field is a record-schema break and
requires a new round per [`../../CONVENTIONS.md`](../../CONVENTIONS.md) §When to cut a new version.

## Host disclosure rules

The harness records `host.cpu`, `host.ram_gb`, and `host.os` into every
record. `RESULTS.md` `Host/environment disclosure` MUST state explicitly
whether all rows in the primary table came from a single host or were
aggregated across hosts.

For v1: aggregation across hosts is **not allowed** in the primary table.
Cross-host data may appear in a separate appendix table but never in the
headline. Once we have a fleet of stable benchmarking hosts, this rule can
relax in a later round.

The reference v1 host is recorded at sweep time. Suggested baseline:
modern x86_64 laptop class (12+ cores, 32+ GB RAM, macOS 25 or Linux 6.x).
The exact reference host is recorded in `RESULTS.md` §5.

## Known caveats

- Cold-cache vs warm-cache effects on the host filesystem can shift `build-cold` numbers by 2x or more. The harness clears the OS page cache before each `build-cold` measurement (or attempts to — kernel permission may force a fallback to `sync` only). This caveat is restated in every `RESULTS.md` §Known caveats.
- Indexer parallelism settings affect build phases. The harness pins parallelism via env var and records the pinned value into each `host` block as `host.parallelism_pin` (a non-required field; consumers ignore it if absent).
- v1 corpora are open-source repositories. They do not exercise proprietary-language syntax extensions or vendored dependencies that some user mono-repos contain.

## Reproduction command

```bash
# Lock corpora to the SHAs this round measured
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-fetch

# Replay the full sweep that produced RESULTS.md (after this round is frozen)
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-sweep

# Regenerate the primary tables from frozen runs/
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-aggregate
```

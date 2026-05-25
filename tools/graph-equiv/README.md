# graph-equiv

`graph-equiv` is the CI equivalence harness for the `orbit-knowledge` v1 to
`orbit-graph` v2 migration described in
`docs/design/orbit-graph/specs/GRAPH_SPEC.md` §16.

The harness reads a frozen corpus from `tools/graph-equiv/corpus/`. There is one
line-oriented selector list per language:

- `rust.txt`
- `typescript.txt`
- `python.txt`
- `go.txt`

Each non-empty, non-comment line starts with one query kind followed by its
argument, for example:

```text
search rust_helper
show symbol:tools/graph-equiv/fixtures/rust/sample.rs#rust_entry:function
refs symbol:tools/graph-equiv/fixtures/rust/sample.rs#rust_helper:function
callees symbol:tools/graph-equiv/fixtures/rust/sample.rs#rust_entry:function
impact symbol:tools/graph-equiv/fixtures/rust/sample.rs#rust_isolated:function
trace py-ship
sync workspace
```

At startup the runner checks the committed corpus checksum. If a selector list
changes without updating the expected checksum in code, the run exits before any
backend query. This keeps corpus drift explicit in review.

## Tolerances

The diff logic implements the five GRAPH_SPEC §16 rules:

- `search <q>` compares the unordered set of `(kind, file, name)` triples. v2
  `string` and `config` extras are ignored; missing v1 symbol matches and extra
  v2 symbol matches fail.
- `show <sel>` compares source bytes byte-for-byte.
- `refs <sym>` compares `(file, line, kind)` triples after v2 is queried at the
  `same_module` confidence floor.
- `callees <sym>` compares `(file, line, target_name)` triples.
- `impact <sym>` compares the depth-3 set of touched symbol qualified names.
- `trace <command>` compares the set of root-to-callee name paths.
- `sync <label>` verifies both backends can refresh before query execution.

Output is a structured JSON report with per-query backend wall-clock timings
and aggregate median/p95 timings. Any out-of-tolerance diff exits non-zero and
includes the language, corpus file line, query kind, selector, tolerance, and
offending rows.

## Running

`make ci-equiv` builds `orbit-graph-cli`, builds `graph-equiv`, then runs:

```sh
cargo run -p graph-equiv -- check --workspace .
```

The v2 backend invokes `orbit-graph-cli` as a subprocess. Set
`ORBIT_GRAPH_CLI=/path/to/orbit-graph-cli` or pass `--orbit-graph-cli PATH` to
test a specific binary.

## Waivers

Per-query waivers live in `bench/equiv-waivers.md`. A waiver is a reviewed
blocker with rationale, owner, selector, query, and planned removal criteria. It
is not a free pass: CI should continue to fail until the waiver has been reviewed
and the follow-up disposition is explicit.

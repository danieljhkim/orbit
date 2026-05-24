# graph-equiv

`graph-equiv` is the scaffold for the `orbit-knowledge` v1 to `orbit-graph`
v2 equivalence harness described in `docs/design/orbit-graph/specs/GRAPH_SPEC.md`
§16.

This crate intentionally lands only the harness shape:

- a backend trait with `search`, `show`, `refs`, `callees`, and `impact`
- a v1 backend wired to the current `orbit-knowledge` command surface for
  `search`, `show`, and `refs`
- a v2 backend whose methods are `unimplemented!("orbit-graph not yet wired")`
- a local smoke command for checking that v1 can query a knowledge graph

`callees` and `impact` are present on the backend trait so P6.1 can fill the
equivalence table without changing the dispatch shape; the v1 backend returns
an unsupported error for them until the exact v1 adapter is added. This scaffold
does not include the frozen selector corpus, per-query diff logic, waivers, a
Make target, or CI enforcement. The harness is not CI-enforced yet. P6.1 is the
follow-on that will add the corpus, equivalence comparison, and CI wiring.

Local checks:

```sh
cargo build -p graph-equiv
cargo test -p graph-equiv
```

Optional v1 smoke check against an existing graph:

```sh
cargo run -p graph-equiv -- smoke --workspace . --query GraphCommandContext
```

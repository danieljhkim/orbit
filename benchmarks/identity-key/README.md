# Identity-Key Benchmark

kind: perf

Empirical benchmark for `LeafNode.identity_key` durability across rebuilds. The
harness creates temporary git repositories, runs `orbit_knowledge::pipeline::run_build`
before and after each mutation, and records what the current source tree does
without asserting that any scenario should pass.

## Running

```bash
make -C benchmarks identity-key-run
```

The command overwrites the v1 run records under
`benchmarks/identity-key/v1/runs/`.

## Output Layout

```text
benchmarks/identity-key/
├── README.md
├── scripts/
│   └── run.sh
└── v1/
    ├── README.md
    ├── METHOD.md
    ├── RESULTS.md
    └── runs/
        ├── rename.json
        ├── move.json
        ├── content_edit.json
        ├── delete_recreate.json
        └── signature_change.json
```

Each JSON record contains the scenario name, mutation, relevant leaves before
and after the mutation, a boolean `preserved` observation, and notes.

## Rounds

| Version | Scope | Report |
|---|---|---|
| [v1](./v1/) | Rust-only five-scenario baseline | [RESULTS.md](./v1/RESULTS.md) |

## Conventions

Layout and versioning rules: [`../CONVENTIONS.md`](../CONVENTIONS.md).

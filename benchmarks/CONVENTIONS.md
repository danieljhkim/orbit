# Benchmark Conventions

The repository retains two runnable suites:

- `identity-key/` is a versioned performance benchmark.
- `knowledge-command-equivalence/` is a byte-stable command-envelope regression
  harness.

## Versioned performance benchmarks

Each top-level `README.md` declares `kind: perf` immediately below its H1. A
version directory captures a complete campaign and becomes immutable when
frozen:

```text
benchmarks/<name>/
├── README.md
├── scripts/
├── v1/
│   ├── README.md
│   ├── METHOD.md
│   └── RESULTS.md
└── v2/
```

Cut a new version when the fixture set, harness behavior, system-under-test pin,
or interpretive frame changes. Frozen results are evidence: correct them through
`CORRECTIONS.md` or a later version rather than rewriting them.

Performance results disclose the task ID, run date, seeds, host, system SHA,
measurement distribution, previous-version delta, caveats, and exact
reproduction command. Cite a versioned `RESULTS.md` plus its commit SHA from
docs and PRs.

## Command-equivalence harnesses

An equivalence harness captures public command JSON from a prepared fixture and
compares later output byte-for-byte. It must:

- copy the fixture to a temporary directory;
- use public command envelopes rather than crate internals;
- avoid mutating the source fixture;
- fail with a unified diff for every mismatch; and
- document capture and compare commands in its README.

## Runs and secrets

Living run directories may be gitignored staging areas. Before freezing or
committing captured output, scan it for API keys, bearer tokens, and other
secrets. Environment-variable names are acceptable; secret values are not.

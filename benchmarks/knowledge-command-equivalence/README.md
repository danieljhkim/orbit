# Knowledge Command Equivalence Harness

This harness captures byte-stable JSON from Orbit's task, search, docs, learning,
and ADR read surfaces for a prepared fixture workspace, then compares a later
run against that baseline. It keeps command-envelope refactors reviewable
without coupling fixtures to implementation crates.

Typical use:

```bash
benchmarks/knowledge-command-equivalence/run.sh capture /path/to/fixture /tmp/orbit-knowledge-baseline
benchmarks/knowledge-command-equivalence/run.sh compare /path/to/fixture /tmp/orbit-knowledge-baseline
```

The fixture workspace should contain representative tasks, docs, learnings, and
ADRs. The script copies the fixture to a temporary directory and runs through
`orbit tool run`, exercising the same public tool envelope agents use without
mutating the fixture.

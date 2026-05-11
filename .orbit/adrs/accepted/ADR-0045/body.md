## Context
Timestamp-only command-audit execution ids collided when concurrent `orbit tool run orbit.task.show` processes in one workspace generated ids at the same effective clock tick.

## Decision
Generate command-audit execution ids through one shared helper that combines a stable prefix, wall-clock nanoseconds, process id, and a per-process atomic sequence while keeping the SQLite unique index authoritative.

## Consequences
- Parallel CLI and runtime audit producers get deterministic collision resistance without weakening uniqueness constraints.
- Cost: execution ids are longer and less visually compact than the old `exec-<nanos>` shape.

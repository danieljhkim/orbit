## Context
CLI commands need durable, filterable history across processes, but full provider payloads would make routine queries noisy and expensive.

## Decision
Keep command audit records as compact SQLite rows with command, target, role, status, timing, working directory, and optional argument/error fields; store transcript detail in JSONL and blobs.

## Consequences
- `orbit audit list/show/stats/export` can stay fast and table-shaped.
- Cost: complete incident reconstruction may require joining command rows with job state and file-backed traces.

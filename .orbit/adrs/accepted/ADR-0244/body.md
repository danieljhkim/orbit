## Context
Bridge needs hybrid Orbit search but can only proxy the dashboard HTTP surface. The alternatives were to keep reconstructing lexical results in Bridge, expose a generic tool-execution HTTP endpoint, or add a narrow search endpoint backed by the same runtime pipeline as the CLI.

## Decision
Expose GET /api/search as a thin transport adapter over OrbitRuntime::global_search. The endpoint accepts the unified query, kind, status, tag, path, hybrid, and semantic parameters and returns the runtime response unchanged, including the effective mode and per-hit retriever rank breakdown. If hybrid infrastructure is unavailable, the shared runtime pipeline degrades to lexical so CLI and HTTP callers observe the same behavior.

## Consequences
- Bridge can proxy one authoritative endpoint instead of owning a second search implementation.
- CLI, tool, and HTTP search share filtering, ranking, result ordering, and fallback semantics.
- Cost: the unified search parameter names and serialized result shape become an HTTP compatibility contract; future search changes must preserve or deliberately version that surface.
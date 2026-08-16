**Context.** ORB-00338 cut the active graph query tools over from `orbit-knowledge` to `orbit-graph`, but audit data and post-cutover testing found unacceptable steady-state regressions: 13.5x p50 search slowdown, a roughly 9s cold-call floor, deleted high-use tools, incomplete plugin MCP exposure, byte-array `show` output, empty `trace` results for real enum-dispatch commands, and direction-confused `impact` output.

**Decision.** Restore the legacy `orbit-knowledge`-backed `orbit.graph.search`, `show`, `refs`, `callers`, `pack`, `overview`, `implementors`, and `deps` surface as the active backend. Keep the `orbit-graph` crate and equivalence harness in tree, but gate any future cutover on the rollback learnings captured in the global ADR.

**Consequences.**
- Future cutover work must use `SyncPolicy::Manual` as the query-tool default unless a measured long-lived process explicitly opts into another policy.
- Pre-cutover audit-log analysis, plugin MCP exposure equivalence, UTF-8 text response boundaries, trace/impact correctness gates, and cold-call latency measurements are required before another backend swap.
- Lost for now: cutover-only `callees`, `impact`, `trace`, the changed `sync` shape, and the extended graph-equiv corpus.
- Cost: **cutover pauses.** The `orbit-graph` backend remains available for development, but agents lose the new cutover-only APIs until the root causes are fixed and a new cutover passes the gates.

- **ADR-0195 — Watcher-backed graph reads** — Accepted.
**Context.** ORB-00377 found that the MCP `orbit.graph.*` read path was effectively poll-on-read: the 500ms `Windowed` policy elapsed between most agent calls, so each query paid for a full worktree diff before running the SQLite lookup. Lengthening the window would reduce frequency but would keep query latency coupled to repository size.

**Decision.** Long-lived MCP graph handles use a watcher-backed policy: `Graph::open` performs one initial auto sync, starts a `notify` watcher scoped to the worktree, coalesces relevant filesystem events behind a debounce, and runs sync in the background. Query methods do not run inline sync for this policy; they read from a cached SQLite connection. The freshness contract is eventual: after a same-process file edit, graph reads may remain stale until the watcher observes and syncs the event, normally within the debounce plus sync duration; callers needing a hard read-after-write barrier must call `Graph::sync`/`orbit.graph.sync` before querying.

**Consequences.**
- Repeated graph reads with no intervening edits are pure SQLite lookups and do not initiate scanner walks.
- Watcher overflow or watcher errors request a coalesced auto sync, preserving the conservative fallback path.
- `Windowed` remains available as an explicit fallback policy, but it is no longer the MCP default.
- Cost: the MCP process now depends on platform filesystem watcher behavior and may serve stale graph data during the documented debounce-plus-sync window.
## Context
orbit-web previously snapshotted ~/.orbit/workspaces.json once at startup and cached runtimes indefinitely, so native workspace init/remove and binding changes required a server restart. A watcher would add a resident process and synchronization surface that the loopback request path does not need.

## Decision
A registry-backed DashboardState reloads the authoritative workspace registry at each request boundary used by the Ws extractor, /api/workspaces, /api/tasks/all, and detailed /healthz. Refresh builds a complete new snapshot, swaps it atomically, and evicts only runtimes for workspaces that were removed, became inactive, or changed binding. A malformed or partial refresh retains the last valid snapshot and emits a credential-safe diagnostic; stale-path entries are reported inactive and are never auto-deleted.

## Consequences
- Native registry mutations become visible without restarting orbit-web.
- Concurrent requests observe either the previous complete snapshot or the next complete snapshot, never a partially rebuilt registry.
- Malformed refreshes remain serviceable from the last known-good snapshot and require operator correction of the registry file.
- Cost: each request boundary re-reads and parses the small registry file, and mutations are eventually consistent with requests already in flight rather than transactionally synchronized with them.
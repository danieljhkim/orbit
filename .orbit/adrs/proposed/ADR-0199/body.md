## Context

MCP host tools (`orbit.task.*`, `orbit.adr.*`, `orbit.learning.*`, `orbit.friction.*`, `orbit.search`) bind to a single `OrbitRuntime` resolved at `serve` launch from cwd discovery; when no initialized workspace is found, the server installs `EmptyMcpHost` and advertises an empty `tools/list`, so clients that launch the server without the repo as cwd lose the entire host surface (e.g. Cowork, which launches with cwd / `CLAUDE_PROJECT_DIR` set to an internal scratchpad; see L-0065, ORB-00405). [ADR-0181] already routes workspace *addressing* per-call → session-context, but tool *registration* still gates on launch discovery. Real alternatives: keep launch-gated registration and require every client to fix cwd; or advertise host tools unconditionally and resolve the runtime per call.

## Decision

Advertise the host tool schemas unconditionally and resolve the target `OrbitRuntime` per call via the [ADR-0181] chain (explicit `workspace_path` → session context → clear missing-workspace error), reusing `OrbitRuntime::try_initialize_existing`. Containment is scoped by surface **by design**: the graph adapter keeps its strict anchor containment ([ORB-00361]) because it indexes the entire worktree and is therefore a path-traversal (CWE-22) read surface; host tools, which only operate on a structured `.orbit/` store, validate only that the resolved path is an initialized Orbit workspace and honor the [ADR-0149] `workspace_id` binding — no traversal anchor is imposed on them.

## Consequences

- `tools/list` returns the full host surface even when launch discovery finds nothing; execution binds to the caller-supplied workspace, making Orbit usable from any MCP client regardless of launch cwd / `CLAUDE_PROJECT_DIR` and removing the per-user `--root` workaround (L-0065).
- The graph adapter's strict containment is retained and explicitly justified as tree-indexing-specific; host tools are documented as exempt, so the asymmetry is intentional rather than an oversight.
- Omitting `workspace_path` preserves current CLI behavior (resolve from cwd / `--root`).
- Cost: the host runtime stops being a single process-lifetime singleton — the server must resolve and cache a runtime per workspace and keep the [ADR-0181] session thread-through correct across that cache, adding per-call resolution state that every future host tool must respect.
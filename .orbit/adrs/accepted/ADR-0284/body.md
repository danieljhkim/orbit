## Context

`orbit web serve` was coupled to a single workspace: the CLI eagerly initialized one `OrbitRuntime` (failing outside a workspace) and handed it to the dashboard as `Arc<OrbitRuntime>` axum state, so 46 of 48 handlers took `State(runtime): State<Arc<OrbitRuntime>>`. Operators wanted one dashboard over every workspace on the machine, launchable from any directory. `~/.orbit/workspaces.json` already enumerates workspaces and the SQLite stores already scope by workspace, so the missing piece was serving many runtimes from one process without rewriting every handler.

## Decision

Introduce `DashboardState` — a workspace-keyed, lazily-built runtime map — as the axum state, and a `Ws` extractor that selects the request's runtime from a `?workspace=<id>` query parameter (falling back to a configured default). Handlers change only their signature line (`State(runtime): State<Arc<OrbitRuntime>>` → `Ws(runtime): Ws`); bodies are untouched. `DashboardState::single` preserves the pre-existing single-workspace behavior (and every handler test). `orbit web serve` dispatches through a new `serve_from_env` before the CLI's eager runtime init, so it works from anywhere: inside a workspace without `--global` it stays single-mode; with `--global` or outside any workspace it enumerates the registry, skipping stale-path entries rather than failing. Two aggregate endpoints (`GET /api/workspaces`, `GET /api/tasks/all`) plus a header workspace selector and an "All workspaces" task view expose the machine-wide surface.

Rejected alternative: workspace-prefixed route paths (`/api/:workspace/tasks`). Rejected because it would rewrite all 48 route registrations and every frontend fetch path, versus a single query-param choke point in `common.js` and one-line handler signature swaps.

## Consequences


- One loopback dashboard can cover every workspace; per-workspace views drill down via the selector while Tasks offers a cross-workspace aggregate.
- The loopback-only bind guard (ORB-00360) is unchanged — global mode broadens data exposure only on the same machine, still with no network binding and no auth added.
- Runtimes are built on first access and cached, so an unopenable or stale workspace degrades to being skipped instead of failing startup.
- Cost: handlers that need a concrete workspace now depend on the `Ws` extractor's selection rules; the aggregate task endpoint reopens each workspace's store per request (no cross-workspace caching of task lists yet).

## Provenance

Migrated verbatim from the local heading `user-interface/ADR-00030` in `docs/design/user-interface/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-07 · [ORB-00030]
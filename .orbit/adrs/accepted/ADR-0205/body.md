## Context

Sweep must find routine definitions without a resident daemon and without bootstrapping from the caller's cwd. Alternatives: a host-level pointer file in `~/.orbit/host.toml` naming one designated control workspace per host (explicit two-way handshake), or reusing the global workspace registry the way `orbit run ship-sweep` / `auto_ship` already does for unattended cross-workspace dispatch.

## Decision

Sweep enumerates `~/.orbit/workspaces.json` and collects `.orbit/routines/*.yaml` from every registered, active workspace whose versioned `.orbit/config.toml` declares `[routines] role = "source"`. Centralizing all routines in polaris is constellation convention, not Orbit mechanism. `~/.orbit/host.toml` survives only to carry `host_id`.

## Consequences

- Setup is what already exists: register the workspace plus one versioned config key; both hosts converge through git with no per-host pointer files.
- `orbit routine list` names each routine's source workspace, so provenance stays unambiguous with multiple sources.
- Cost: any registered workspace's config can make it a routine source — the review boundary widens from one blessed repo to every registered workspace's `config.toml`, and sweep correctness now depends on registry hygiene (stale registered paths must be skipped loudly, not silently).
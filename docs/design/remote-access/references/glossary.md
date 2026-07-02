# Glossary: Remote Access

Vocabulary specific to how Orbit is viewed across workspaces and machines. Standard terms (loopback, SSH port forwarding, pty, TOCTOU) are used with their ordinary meaning and excluded here unless remote access gives them a specific role.

| Term | Meaning |
|------|---------|
| **Aggregate view** | The "All workspaces" task list backed by `GET /api/tasks/all`, tagging each task with its workspace. Per-machine, not cross-machine. See [2_design.md §2](../2_design.md). |
| **connect** | `orbit web connect <ssh-host>`: the client-side command that tunnels a remote machine's loopback dashboard over SSH. See [2_design.md §3](../2_design.md). |
| **Global mode** | `orbit web serve --global` (or serve run outside any workspace): serves every workspace in `~/.orbit/workspaces.json`. Contrast single mode. See [2_design.md §1](../2_design.md). |
| **Single mode** | `orbit web serve` inside a workspace without `--global`: serves exactly that workspace, preserving pre-[ORB-00030] behavior. See [2_design.md §1](../2_design.md). |
| **`Ws` extractor** | The axum extractor that selects a request's runtime from `?workspace=<id>` (or the default), replacing the former single-runtime `State`. See [2_design.md §2](../2_design.md). |
| **WsEntry** | One servable workspace in `DashboardState` (`id`, `name`, `repo_root`, `orbit_dir`, `active`); inactive entries are listed but never built. See [2_design.md §2](../2_design.md). |
| **Viewing-not-sync** | The load-bearing boundary: remote access shows existing state on a reachable machine and never synchronizes or writes across machines. See [4_decisions.md ADR-0200](../4_decisions.md). |
